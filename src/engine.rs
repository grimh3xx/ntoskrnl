//! Aplicar, desfazer e conferir tweaks. Porta do motor do CTS Optimizer:
//! backup gravado antes de mudar, conferência depois (o Windows às vezes ignora a mudança em silêncio)
//! e desfazer automático do que deu tempo de aplicar quando algo falha.

use crate::catalog::{tr, Catalog, CmdAction, RegAction, Tweak};
use crate::state::{now, Backup, State};
use crate::sys::{proc, registry, SysInfo};
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::time::Duration;

const BACKUP_ENV: &str = "NTOSKRNL_BACKUP";

#[derive(Clone, Debug)]
pub struct TweakStatus {
    pub applied: bool,
    /// tem backup: dá para desfazer
    pub revertible: bool,
    /// motivo de não servir para este PC
    pub blocked: Option<String>,
}

/// Resultado de uma ação sobre um item: Ok ou a mensagem de erro
pub type Outcome = (String, Result<(), String>);

/// Não serve para este PC (GPU, SSD, build, RAM, notebook, edição, Defender)
pub fn incompatibility(t: &Tweak, s: &SysInfo) -> Option<String> {
    let r = t.requires.as_ref()?;
    if let Some(g) = &r.gpu {
        if s.gpu_vendor() != g {
            return Some(format!("{} {}", tr("Só para placas", "Only for"), g.to_uppercase()));
        }
    }
    if r.ssd && !s.ssd {
        return Some(tr("Só quando o Windows está em SSD", "Only when Windows is on an SSD").into());
    }
    if let Some(b) = r.min_build {
        if s.build < b {
            return Some(tr("Precisa de um Windows mais novo", "Requires a newer Windows").into());
        }
    }
    if let Some(gb) = r.min_ram_gb {
        if s.ram_gb < gb - 0.5 {
            return Some(format!("{} {gb} GB {}", tr("Só com", "Only with"), tr("de RAM ou mais", "RAM or more")));
        }
    }
    if r.desktop_only && s.laptop {
        return Some(tr("Não recomendado em notebook", "Not recommended on laptops").into());
    }
    if r.not_home && s.is_home() {
        return Some(tr("Não existe no Windows Home", "Not available on Windows Home").into());
    }
    if r.home_only && !s.is_home() {
        return Some(tr("Seu Windows já vem com isso (só o Home não tem)", "Your Windows already has it (only Home lacks it)").into());
    }
    if r.defender && !s.defender {
        return Some(tr("Precisa do Microsoft Defender ativo", "Requires Microsoft Defender active").into());
    }
    None
}

fn regs_match(regs: &[RegAction]) -> bool {
    regs.iter().all(|a| registry::read(&a.path, &a.name).matches(&a.value))
}

pub fn status(c: &Catalog, st: &State, s: &SysInfo) -> BTreeMap<String, TweakStatus> {
    c.tweaks
        .iter()
        .map(|t| {
            let regs = t.reg_actions();
            let backup = st.backups.contains_key(&t.id);
            // tweaks de comando não têm leitura rápida: vale o backup de que o ntoskrnl aplicou
            let applied = if t.cmd.is_empty() { !regs.is_empty() && regs_match(&regs) } else { backup && regs_match(&regs) };
            (t.id.clone(), TweakStatus { applied, revertible: backup, blocked: incompatibility(t, s) })
        })
        .collect()
}

/// Roda o script de uma ação. Só erro de verdade (throw) vira falha: o powershell.exe sai com 1 quando o
/// último comando teve erro silenciado, e isso não quer dizer nada. Quem diz se pegou é o verify.
fn run_cmd(script: &str, backup: &str, timeout: Duration) -> Result<String> {
    let wrapped = format!(
        "try {{\n& {{\n$ErrorActionPreference = 'Continue'\n{script}\n}}\n}} catch {{ [Console]::Error.WriteLine('NFERR:' + $_.Exception.Message); exit 1 }}\nexit 0"
    );
    let o = proc::ps(&wrapped, &[(BACKUP_ENV, backup)], timeout)?;
    if o.code != 0 {
        let msg = o.stderr.lines().find_map(|l| l.strip_prefix("NFERR:")).map(str::trim).unwrap_or("");
        if msg.is_empty() {
            bail!("{} ({})", tr("o script falhou sem mensagem", "the script failed with no message"), o.code);
        }
        bail!("{msg}");
    }
    Ok(o.stdout.trim().to_string())
}

fn timeout_of(c: &CmdAction) -> Duration {
    Duration::from_secs(c.timeout_secs.unwrap_or(300))
}

/// Relê tudo depois de aplicar: Proteção contra Adulteração, políticas e drivers ignoram mudanças em silêncio
fn verify(t: &Tweak, b: &Backup) -> Result<()> {
    let off: Vec<_> = t.reg_actions().into_iter().filter(|a| !registry::read(&a.path, &a.name).matches(&a.value)).map(|a| a.name).collect();
    if !off.is_empty() {
        bail!("{} ({})", tr("O Windows não aceitou a mudança", "Windows did not accept the change"), off.join(", "));
    }
    for c in &t.cmd {
        let Some(v) = &c.verify else { continue };
        let out = run_cmd(v, b.cmd.get(&c.key).map_or("", |s| s), timeout_of(c))?;
        let last = out.lines().last().unwrap_or("").trim();
        if cfg!(windows) && last != "ok" {
            bail!("{}{}", tr("O Windows não aceitou a mudança", "Windows did not accept the change"), if last.is_empty() { String::new() } else { format!(": {last}") });
        }
    }
    Ok(())
}

fn revert_one(t: &Tweak, b: &Backup) -> Result<()> {
    // ordem inversa da aplicação; só valores que o próprio tweak mexe
    let own = t.reg_actions();
    for snap in b.registry.iter().rev() {
        if own.iter().any(|a| snap.same_key(&a.path, &a.name)) {
            registry::restore(snap)?;
        }
    }
    // serviço que estava automático volta a rodar agora, sem esperar reiniciar
    for s in &t.service {
        let path = format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{}", s.name);
        if b.registry.iter().any(|x| x.same_key(&path, "Start") && x.matches(&crate::catalog::RegValue::Num(2))) {
            let _ = proc::run("sc.exe", &["start", &s.name], &[], Duration::from_secs(30), |_| {});
        }
    }
    for c in t.cmd.iter().rev() {
        run_cmd(&c.revert, b.cmd.get(&c.key).map_or("", |s| s), timeout_of(c))?;
    }
    Ok(())
}

/// Outros tweaks aplicados que gravam algum dos mesmos valores: são desfeitos antes, senão o backup
/// do novo guardaria o valor que o outro gravou em vez do original
fn overlapping<'a>(c: &'a Catalog, st: &State, t: &Tweak) -> Vec<&'a Tweak> {
    let mine = t.reg_actions();
    st.backups
        .keys()
        .filter(|id| **id != t.id)
        .filter_map(|id| c.tweak(id))
        .filter(|o| o.reg_actions().iter().any(|a| mine.iter().any(|m| m.path.eq_ignore_ascii_case(&a.path) && m.name.eq_ignore_ascii_case(&a.name))))
        .collect()
}

fn apply_one(c: &Catalog, st: &mut State, s: &SysInfo, t: &Tweak, log: &mut dyn FnMut(String)) -> Result<()> {
    if let Some(why) = incompatibility(t, s) {
        bail!("{why}");
    }
    let is_new = !st.backups.contains_key(&t.id);
    let backup = match st.backups.get(&t.id).cloned() {
        // reaplicar nunca sobrescreve o backup original; só guarda valores que uma versão nova acrescentou
        Some(mut b) => {
            for a in t.reg_actions() {
                if !b.registry.iter().any(|x| x.same_key(&a.path, &a.name)) {
                    b.registry.push(registry::read(&a.path, &a.name));
                }
            }
            st.backups.insert(t.id.clone(), b.clone());
            st.save()?;
            b
        }
        None => {
            for o in overlapping(c, st, t) {
                log(format!("  {} {}", tr("desfazendo antes:", "reverting first:"), o.title.get()));
                revert_one(o, &st.backups[&o.id])?;
                st.backups.remove(&o.id);
            }
            let registry = t.reg_actions().iter().map(|a| registry::read(&a.path, &a.name)).collect();
            let mut cmd = BTreeMap::new();
            for a in &t.cmd {
                if let Some(script) = &a.backup {
                    cmd.insert(a.key.clone(), run_cmd(script, "", timeout_of(a))?);
                }
            }
            let b = Backup { applied_at: now(), registry, cmd };
            // o backup vai para o disco ANTES de mexer em qualquer coisa
            st.backups.insert(t.id.clone(), b.clone());
            st.save()?;
            b
        }
    };

    let result = (|| -> Result<()> {
        for a in t.reg_actions() {
            registry::set(&a.path, &a.name, &a.kind, &a.value)?;
        }
        for sv in &t.service {
            if sv.start == crate::catalog::StartMode::Disabled {
                let _ = proc::run("sc.exe", &["stop", &sv.name], &[], Duration::from_secs(30), |_| {});
            }
        }
        for a in &t.cmd {
            run_cmd(&a.apply, backup.cmd.get(&a.key).map_or("", |s| s), timeout_of(a))?;
        }
        verify(t, &backup)
    })();

    if let Err(e) = result {
        // desfaz o que deu tempo de aplicar, se foi esta aplicação que criou o backup
        if is_new {
            let _ = revert_one(t, &backup);
            st.backups.remove(&t.id);
            st.save()?;
        }
        return Err(e);
    }
    Ok(())
}

pub fn apply(c: &Catalog, st: &mut State, s: &SysInfo, ids: &[String], log: &mut dyn FnMut(String)) -> Vec<Outcome> {
    let mut out = Vec::new();
    for id in ids {
        let Some(t) = c.tweak(id) else {
            out.push((id.clone(), Err(tr("tweak desconhecido", "unknown tweak").into())));
            continue;
        };
        log(format!("{} {}", tr("Aplicando", "Applying"), t.title.get()));
        let r = apply_one(c, st, s, t, log).map_err(|e| e.to_string());
        match &r {
            Ok(()) => log(format!("  ok{}", if t.restart { tr(" (reinicie para valer)", " (restart to take effect)") } else { "" })),
            Err(e) => log(format!("  {}: {e}", tr("falhou", "failed"))),
        }
        out.push((id.clone(), r));
    }
    out
}

pub fn revert(c: &Catalog, st: &mut State, ids: &[String], log: &mut dyn FnMut(String)) -> Vec<Outcome> {
    let mut out = Vec::new();
    for id in ids {
        let (Some(t), Some(b)) = (c.tweak(id), st.backups.get(id).cloned()) else {
            out.push((id.clone(), Err(tr("sem backup: não foi aplicado pelo ntoskrnl", "no backup: not applied by ntoskrnl").into())));
            continue;
        };
        log(format!("{} {}", tr("Desfazendo", "Reverting"), t.title.get()));
        let r = revert_one(t, &b).and_then(|_| {
            st.backups.remove(id);
            st.save()
        });
        let r = r.map_err(|e| e.to_string());
        match &r {
            Ok(()) => log("  ok".into()),
            Err(e) => log(format!("  {}: {e}", tr("falhou", "failed"))),
        }
        out.push((id.clone(), r));
    }
    out
}

/// Tweaks aplicados que o Windows desfez (as atualizações grandes voltam alguns ao padrão sem avisar).
/// Valores sem backup ainda (acrescentados numa versão nova do catálogo) não contam.
pub fn drift(c: &Catalog, st: &State) -> Vec<String> {
    let mut off = Vec::new();
    for (id, b) in &st.backups {
        let Some(t) = c.tweak(id) else { continue };
        let reg_off = t
            .reg_actions()
            .iter()
            .filter(|a| b.registry.iter().any(|x| x.same_key(&a.path, &a.name)))
            .any(|a| !registry::read(&a.path, &a.name).matches(&a.value));
        let cmd_off = !reg_off
            && cfg!(windows)
            && t.cmd.iter().any(|a| {
                a.verify.as_ref().is_some_and(|v| {
                    run_cmd(v, b.cmd.get(&a.key).map_or("", |s| s), timeout_of(a)).map(|o| o.lines().last().unwrap_or("").trim() != "ok").unwrap_or(true)
                })
            });
        if reg_off || cmd_off {
            off.push(id.clone());
        }
    }
    off
}

/// O Windows só deixa criar 1 ponto a cada 24h: libera durante a criação e volta ao padrão
pub fn restore_point() -> Result<()> {
    let o = proc::ps(
        r#"
$k = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore'
try {
  Set-ItemProperty $k -Name SystemRestorePointCreationFrequency -Value 0 -Type DWord
  Enable-ComputerRestore -Drive "$env:SystemDrive\"
  Checkpoint-Computer -Description 'ntoskrnl' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop
} finally {
  Remove-ItemProperty $k -Name SystemRestorePointCreationFrequency -ErrorAction SilentlyContinue
}"#,
        &[],
        Duration::from_secs(300),
    )?;
    if o.code != 0 {
        bail!("{}", proc::first_line(&o.stderr));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    // só no registro simulado: no Windows mexeria no registro de verdade de quem roda os testes
    #[cfg(not(windows))]
    #[test]
    fn apply_then_revert_restores_original() {
        std::env::set_var("NTOSKRNL_DATA", std::env::temp_dir().join("ntoskrnl-test"));
        let c = Catalog::load(None).unwrap();
        let s = crate::sys::sysinfo();
        let mut st = State::default();
        let id = "mouse-accel-off".to_string();
        let t = c.tweak(&id).unwrap();
        let before: Vec<_> = t.reg_actions().iter().map(|a| registry::read(&a.path, &a.name).exists).collect();

        let r = apply(&c, &mut st, &s, std::slice::from_ref(&id), &mut |_| {});
        assert!(r[0].1.is_ok(), "{:?}", r[0].1);
        assert!(status(&c, &st, &s)[&id].applied);

        let r = revert(&c, &mut st, std::slice::from_ref(&id), &mut |_| {});
        assert!(r[0].1.is_ok());
        let after: Vec<_> = t.reg_actions().iter().map(|a| registry::read(&a.path, &a.name).exists).collect();
        assert_eq!(before, after);
        assert!(!st.backups.contains_key(&id));
    }

    #[test]
    fn requirements_block_wrong_gpu() {
        let c = Catalog::load(None).unwrap();
        let mut s = crate::sys::sysinfo();
        s.gpus = "AMD Radeon RX 7800 XT".into();
        assert!(incompatibility(c.tweak("nvidia-hdcp-off").unwrap(), &s).is_some());
        assert!(incompatibility(c.tweak("amd-ulps-off").unwrap(), &s).is_none());
    }
}
