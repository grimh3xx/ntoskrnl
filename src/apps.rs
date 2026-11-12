//! Programas pelo winget (vem no Windows 10/11) e remoção de apps pré-instalados (AppX).

use crate::catalog::{tr, App, Bloat};
use crate::engine::Outcome;
use crate::sys::{data_dir, proc};
use std::collections::HashSet;
use std::time::Duration;

const WINGET_FLAGS: [&str; 2] = ["--accept-source-agreements", "--disable-interactivity"];

/// Códigos de saída do winget que também querem dizer "o programa está no PC"
/// (doc/windows/package-manager/winget/returnCodes.md no winget-cli)
const WINGET_OK: [u32; 6] = [0, 0x8A15_002B, 0x8A15_0061, 0x8A15_010D, 0x8A15_010E, 0x8A15_0109];

fn winget_reason(code: u32) -> Option<&'static str> {
    Some(match code {
        0x8A15_0008 => tr("falha no download", "download failed"),
        0x8A15_0011 => tr("hash do instalador não confere", "installer hash mismatch"),
        0x8A15_0014 => tr("pacote não encontrado no winget", "package not found in winget"),
        0x8A15_003A | 0x8A15_010F => tr("bloqueado por política do PC", "blocked by a PC policy"),
        0x8A15_0056 => tr("o instalador não aceita rodar como administrador", "the installer refuses to run elevated"),
        0x8A15_0101 | 0x8A15_0103 => tr("o programa está aberto: feche e tente de novo", "the app is running: close it and retry"),
        0x8A15_0102 => tr("outra instalação em andamento", "another install is in progress"),
        0x8A15_0105 => tr("sem espaço em disco", "not enough disk space"),
        0x8A15_0107 => tr("sem internet", "no internet"),
        0x8A15_010A => tr("reinicie o PC antes", "restart the PC first"),
        _ => return None,
    })
}

/// Ids instalados pelo `winget export` (o `winget list` corta ids compridos com "…")
pub fn installed_apps() -> HashSet<String> {
    if !cfg!(windows) {
        return ["Valve.Steam", "Mozilla.Firefox", "7zip.7zip"].iter().map(|s| s.to_ascii_lowercase()).collect();
    }
    let _ = std::fs::create_dir_all(data_dir());
    let file = data_dir().join("winget-export.json");
    let f = file.to_string_lossy().into_owned();
    let mut args = vec!["export", "-o", f.as_str(), "--include-versions"];
    args.extend(WINGET_FLAGS);
    let _ = proc::run("winget.exe", &args, &[], Duration::from_secs(120), |_| {});
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let _ = std::fs::remove_file(&file);
    let v: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_default();
    v["Sources"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["Packages"].as_array().cloned().unwrap_or_default())
        .filter_map(|p| p["PackageIdentifier"].as_str().map(str::to_ascii_lowercase))
        .collect()
}

pub fn winget_available() -> bool {
    !cfg!(windows) || proc::run("winget.exe", &["--version"], &[], Duration::from_secs(20), |_| {}).is_ok_and(|o| o.code == 0)
}

pub fn install(apps: &[&App], log: &mut dyn FnMut(String)) -> Vec<Outcome> {
    apps.iter()
        .map(|a| {
            log(format!("{} {} ({})", tr("Instalando", "Installing"), a.name, a.id));
            let mut args = vec!["install", "--id", a.id.as_str(), "--exact", "--silent", "--accept-package-agreements"];
            if let Some(src) = &a.source {
                args.extend(["--source", src.as_str()]);
            }
            args.extend(WINGET_FLAGS);
            (a.id.clone(), winget(&args, log))
        })
        .collect()
}

/// Atualiza tudo que o winget conhece (não só o que veio do catálogo)
pub fn upgrade_all(log: &mut dyn FnMut(String)) -> Vec<Outcome> {
    log(tr("Atualizando todos os programas (winget upgrade --all)", "Upgrading every app (winget upgrade --all)").into());
    let mut args = vec!["upgrade", "--all", "--silent", "--accept-package-agreements", "--include-unknown"];
    args.extend(WINGET_FLAGS);
    vec![("upgrade --all".into(), winget(&args, log))]
}

fn winget(args: &[&str], log: &mut dyn FnMut(String)) -> Result<(), String> {
    let mut last = String::new();
    let o = proc::run("winget.exe", args, &[], Duration::from_secs(1800), |l| {
        let l = l.trim();
        // a barra de progresso vira muitas linhas: só as que têm texto de verdade vão para o log
        if l.chars().any(char::is_alphabetic) && !l.contains('█') && !l.contains('▒') {
            log(format!("  {l}"));
            last = l.to_string();
        }
    })
    .map_err(|e| e.to_string())?;
    let code = o.code as u32;
    if WINGET_OK.contains(&code) {
        log("  ok".into());
        return Ok(());
    }
    let why = winget_reason(code).map(String::from).unwrap_or_else(|| format!("winget 0x{code:08X}: {last}"));
    log(format!("  {}: {why}", tr("falhou", "failed")));
    Err(why)
}

/// Nomes dos pacotes AppX instalados (para o usuário atual ou provisionados para novos usuários)
pub fn installed_bloat() -> HashSet<String> {
    if !cfg!(windows) {
        return ["Microsoft.BingNews", "Microsoft.BingWeather", "Microsoft.Getstarted", "Clipchamp.Clipchamp", "Microsoft.GamingApp"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    }
    let names: Vec<String> = proc::ps_json(
        "@(@(Get-AppxPackage -AllUsers).Name + @(Get-AppxProvisionedPackage -Online).DisplayName | Sort-Object -Unique) | ConvertTo-Json -Compress",
        Duration::from_secs(120),
    )
    .unwrap_or_default();
    names.into_iter().collect()
}

/// Remove de todos os usuários e também o pacote provisionado, senão volta para cada usuário novo.
/// Só ids da lista fechada do catálogo chegam aqui.
pub fn remove_bloat(items: &[&Bloat], log: &mut dyn FnMut(String)) -> Vec<Outcome> {
    items
        .iter()
        .map(|b| {
            log(format!("{} {}", tr("Removendo", "Removing"), b.name));
            let script = format!(
                "$n = '{}'\n\
                 Get-AppxPackage -AllUsers -Name $n | Remove-AppxPackage -AllUsers -ErrorAction SilentlyContinue\n\
                 Get-AppxProvisionedPackage -Online | Where-Object DisplayName -eq $n | Remove-AppxProvisionedPackage -Online -ErrorAction SilentlyContinue | Out-Null\n\
                 if (Get-AppxPackage -AllUsers -Name $n) {{ 'still' }} else {{ 'ok' }}",
                b.id.replace('\'', "''")
            );
            let r = match proc::ps(&script, &[], Duration::from_secs(300)) {
                Ok(o) if o.stdout.trim().ends_with("ok") => Ok(()),
                Ok(_) => Err(tr("o Windows não deixou remover (talvez esteja aberto)", "Windows refused to remove it (maybe it is open)").into()),
                Err(e) => Err(e.to_string()),
            };
            log(match &r {
                Ok(()) => "  ok".into(),
                Err(e) => format!("  {}: {e}", tr("falhou", "failed")),
            });
            (b.id.clone(), r)
        })
        .collect()
}
