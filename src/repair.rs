//! Reparos do Windows: só ferramentas da própria Microsoft, com a saída ao vivo no log.

use crate::catalog::tr;
use crate::sys::proc;
use anyhow::{bail, Result};
use std::time::Duration;

pub struct Repair {
    pub id: &'static str,
    pub title: (&'static str, &'static str),
    pub description: (&'static str, &'static str),
    pub restart: bool,
}

impl Repair {
    pub fn title(&self) -> &'static str {
        tr(self.title.0, self.title.1)
    }
    pub fn description(&self) -> &'static str {
        tr(self.description.0, self.description.1)
    }
}

pub const REPAIRS: &[Repair] = &[
    Repair {
        id: "restore-point",
        title: ("Criar ponto de restauração", "Create restore point"),
        description: ("Foto do sistema para voltar pela Restauração do Sistema se algo der errado.", "A system snapshot you can roll back to with System Restore."),
        restart: false,
    },
    Repair {
        id: "sfc",
        title: ("Verificar arquivos do sistema (SFC)", "Check system files (SFC)"),
        description: ("sfc /scannow: confere e repara arquivos corrompidos do Windows. Leva de 5 a 15 minutos.", "sfc /scannow: checks and repairs corrupted Windows files. Takes 5 to 15 minutes."),
        restart: false,
    },
    Repair {
        id: "dism",
        title: ("Reparar imagem do Windows (DISM)", "Repair Windows image (DISM)"),
        description: ("DISM /RestoreHealth: baixa do Windows Update o que estiver estragado na imagem. Rode antes do SFC se ele não conseguir reparar.", "DISM /RestoreHealth: downloads from Windows Update whatever is broken in the image. Run before SFC if it cannot repair."),
        restart: false,
    },
    Repair {
        id: "network",
        title: ("Resetar a rede", "Reset network"),
        description: ("Limpa o cache de DNS e reseta Winsock e TCP/IP. Resolve internet que conecta mas não abre nada.", "Flushes DNS and resets Winsock and TCP/IP. Fixes internet that connects but loads nothing."),
        restart: true,
    },
    Repair {
        id: "wu-cache",
        title: ("Destravar o Windows Update", "Unstick Windows Update"),
        description: ("Para o Windows Update, apaga os downloads presos em SoftwareDistribution e liga de novo.", "Stops Windows Update, deletes stuck downloads in SoftwareDistribution and starts it again."),
        restart: false,
    },
    Repair {
        id: "temp",
        title: ("Limpar arquivos temporários", "Clean temporary files"),
        description: ("Apaga o %TEMP% do usuário e o C:\\Windows\\Temp. Arquivos em uso ficam.", "Deletes the user %TEMP% and C:\\Windows\\Temp. Files in use are kept."),
        restart: false,
    },
];

pub fn find(id: &str) -> Option<&'static Repair> {
    REPAIRS.iter().find(|r| r.id == id)
}

fn exe(cmd: &str, args: &[&str], log: &mut dyn FnMut(String)) -> Result<()> {
    log(format!("> {cmd} {}", args.join(" ")));
    let o = proc::run(cmd, args, &[], Duration::from_secs(3600), |l| {
        let l = l.trim();
        if !l.is_empty() {
            log(format!("  {l}"));
        }
    })?;
    if o.code != 0 {
        bail!("{cmd} {} {}", tr("saiu com código", "exited with code"), o.code);
    }
    Ok(())
}

fn ps(script: &str, log: &mut dyn FnMut(String)) -> Result<()> {
    let o = proc::ps(script, &[], Duration::from_secs(600))?;
    for l in o.stdout.lines().filter(|l| !l.trim().is_empty()) {
        log(format!("  {l}"));
    }
    if o.code != 0 {
        bail!("{}", proc::first_line(&o.stderr));
    }
    Ok(())
}

pub fn run(id: &str, log: &mut dyn FnMut(String)) -> Result<()> {
    match id {
        "restore-point" => crate::engine::restore_point(),
        "sfc" => exe("sfc.exe", &["/scannow"], log),
        "dism" => exe("DISM.exe", &["/Online", "/Cleanup-Image", "/RestoreHealth"], log),
        "network" => {
            exe("ipconfig.exe", &["/flushdns"], log)?;
            exe("netsh.exe", &["winsock", "reset"], log)?;
            exe("netsh.exe", &["int", "ip", "reset"], log)
        }
        "wu-cache" => ps(
            r#"
$svc = 'wuauserv', 'bits', 'cryptsvc'
Stop-Service $svc -Force -ErrorAction SilentlyContinue
$d = Join-Path $env:SystemRoot 'SoftwareDistribution\Download'
$n = @(Get-ChildItem $d -Force -ErrorAction SilentlyContinue).Count
Remove-Item (Join-Path $d '*') -Recurse -Force -ErrorAction SilentlyContinue
Start-Service $svc -ErrorAction SilentlyContinue
"$n itens removidos de $d""#,
            log,
        ),
        "temp" => ps(
            r#"
$total = 0
foreach ($d in @($env:TEMP, (Join-Path $env:SystemRoot 'Temp'))) {
  Get-ChildItem $d -Force -ErrorAction SilentlyContinue | ForEach-Object {
    $size = if ($_.PSIsContainer) { [int64](Get-ChildItem $_.FullName -Recurse -Force -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum } else { $_.Length }
    Remove-Item $_.FullName -Recurse -Force -ErrorAction SilentlyContinue
    if (-not (Test-Path $_.FullName)) { $total += $size }
  }
}
'{0:N1} MB liberados' -f ($total / 1MB)"#,
            log,
        ),
        _ => bail!("{} {id}", tr("reparo desconhecido:", "unknown repair:")),
    }
}
