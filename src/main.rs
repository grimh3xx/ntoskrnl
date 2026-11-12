//! ntoskrnl — caixa de ferramentas do Windows pelo terminal.
//!
//! Sem argumentos abre a TUI; com subcomando roda direto (scripts, automação, pós-formatação).
//! Todo tweak guarda o valor original antes de mudar e pode ser desfeito.

mod apps;
mod catalog;
mod engine;
mod repair;
mod state;
mod sys;
mod tui;

use anyhow::{bail, Result};
use catalog::{tr, Catalog, Lang};
use clap::{Parser, Subcommand};
use state::State;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ntoskrnl", version, about = "Windows toolbox for the terminal: tweaks with real undo, apps, debloat and repairs")]
struct Cli {
    /// Idioma da interface (pt ou en)
    #[arg(long, global = true, default_value = "pt")]
    lang: String,
    /// Usa os .toml desta pasta no lugar do catálogo embutido
    #[arg(long, global = true)]
    catalog: Option<PathBuf>,
    /// Não cria ponto de restauração antes de aplicar
    #[arg(long, global = true)]
    no_restore_point: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Lista os tweaks com o estado neste PC
    List {
        #[arg(long)]
        json: bool,
    },
    /// Aplica tweaks pelo id
    Apply { ids: Vec<String> },
    /// Desfaz tweaks pelo id (--all desfaz tudo que o ntoskrnl aplicou)
    Revert {
        ids: Vec<String>,
        #[arg(long)]
        all: bool,
    },
    /// Aplica (ou desfaz com --revert) um perfil: recomendado, gamer, privacidade, blindagem, minimo
    Profile {
        name: String,
        #[arg(long)]
        revert: bool,
    },
    /// Instala programas pelo id do winget (só os do catálogo)
    Install { ids: Vec<String> },
    /// Atualiza todos os programas pelo winget
    Upgrade,
    /// Remove apps pré-instalados pelo nome do pacote (só os da lista do catálogo)
    Debloat { ids: Vec<String> },
    /// Roda um reparo: restore-point, sfc, dism, network, wu-cache, temp
    Repair { id: String },
    /// Mostra tweaks que o Windows desfez sozinho (--fix reaplica)
    Check {
        #[arg(long)]
        fix: bool,
    },
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("ntoskrnl: {e:#}");
        std::process::exit(1);
    }
}

fn print_log(l: String) {
    println!("{l}");
}

fn report(r: &[engine::Outcome]) -> Result<()> {
    let bad = r.iter().filter(|x| x.1.is_err()).count();
    println!("\n{} ok, {} {}", r.len() - bad, bad, tr("com falha", "failed"));
    if bad > 0 {
        std::process::exit(2);
    }
    Ok(())
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    catalog::set_lang(if cli.lang.eq_ignore_ascii_case("en") { Lang::En } else { Lang::Pt });
    let cat = Catalog::load(cli.catalog.as_deref())?;
    let Some(cmd) = cli.cmd else {
        return tui::run(cat, !cli.no_restore_point);
    };

    let mut st = State::load();
    let restore_point = |log: &mut dyn FnMut(String)| {
        if !cli.no_restore_point {
            log(tr("Criando ponto de restauração...", "Creating restore point...").into());
            if let Err(e) = engine::restore_point() {
                log(format!("  {}: {e}", tr("não deu (seguindo mesmo assim)", "failed (continuing anyway)")));
            }
        }
    };

    match cmd {
        Cmd::List { json } => {
            let s = sys::sysinfo();
            let status = engine::status(&cat, &st, &s);
            if json {
                let v: Vec<_> = cat
                    .tweaks
                    .iter()
                    .map(|t| {
                        let x = &status[&t.id];
                        serde_json::json!({
                            "id": t.id, "category": t.category, "risk": t.risk.label(), "title": t.title.get(),
                            "applied": x.applied, "revertible": x.revertible, "blocked": x.blocked,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!("{}\n", s.os_name());
                for t in &cat.tweaks {
                    let x = &status[&t.id];
                    let mark = if x.applied { "[x]" } else if x.blocked.is_some() { " - " } else { "[ ]" };
                    println!("{mark} {:<30} {:<9} {}", t.id, t.risk.label(), t.title.get());
                }
            }
        }
        Cmd::Apply { ids } => {
            if ids.is_empty() {
                bail!("{}", tr("diga quais tweaks (veja `ntoskrnl list`)", "name the tweaks (see `ntoskrnl list`)"));
            }
            let s = sys::sysinfo();
            restore_point(&mut print_log);
            report(&engine::apply(&cat, &mut st, &s, &ids, &mut print_log))?;
        }
        Cmd::Revert { ids, all } => {
            let ids = if all { st.backups.keys().cloned().collect() } else { ids };
            report(&engine::revert(&cat, &mut st, &ids, &mut print_log))?;
        }
        Cmd::Profile { name, revert } => {
            let Some(p) = cat.profile(&name) else {
                let names: Vec<_> = cat.profiles.iter().map(|p| p.id.as_str()).collect();
                bail!("{} {name} ({})", tr("perfil desconhecido:", "unknown profile:"), names.join(", "));
            };
            if revert {
                let ids: Vec<_> = p.tweaks.iter().filter(|id| st.backups.contains_key(*id)).cloned().collect();
                return report(&engine::revert(&cat, &mut st, &ids, &mut print_log));
            }
            let s = sys::sysinfo();
            // o que não serve para este PC fica de fora em silêncio (é esperado num perfil)
            let ids: Vec<_> = p.tweaks.iter().filter(|id| engine::incompatibility(cat.tweak(id).unwrap(), &s).is_none()).cloned().collect();
            restore_point(&mut print_log);
            let mut r = engine::apply(&cat, &mut st, &s, &ids, &mut print_log);
            let bloat: Vec<_> = cat.bloat.iter().filter(|b| p.debloat.contains(&b.id)).collect();
            if !bloat.is_empty() {
                let present = apps::installed_bloat();
                let bloat: Vec<_> = bloat.into_iter().filter(|b| present.contains(&b.id)).collect();
                r.extend(apps::remove_bloat(&bloat, &mut print_log));
            }
            report(&r)?;
        }
        Cmd::Install { ids } => {
            let list: Vec<_> = ids.iter().map(|id| cat.apps.iter().find(|a| a.id.eq_ignore_ascii_case(id)).ok_or_else(|| anyhow::anyhow!("{} {id}", tr("app fora do catálogo:", "app not in catalog:")))).collect::<Result<_>>()?;
            report(&apps::install(&list, &mut print_log))?;
        }
        Cmd::Upgrade => report(&apps::upgrade_all(&mut print_log))?,
        Cmd::Debloat { ids } => {
            let list: Vec<_> = ids.iter().map(|id| cat.bloat.iter().find(|b| b.id.eq_ignore_ascii_case(id)).ok_or_else(|| anyhow::anyhow!("{} {id}", tr("fora da lista de debloat:", "not in the debloat list:")))).collect::<Result<_>>()?;
            report(&apps::remove_bloat(&list, &mut print_log))?;
        }
        Cmd::Repair { id } => {
            let r = repair::find(&id).ok_or_else(|| anyhow::anyhow!("{} {id}", tr("reparo desconhecido:", "unknown repair:")))?;
            println!("{}", r.title());
            repair::run(&id, &mut print_log)?;
            println!("ok{}", if r.restart { tr(" (reinicie o PC)", " (restart the PC)") } else { "" });
        }
        Cmd::Check { fix } => {
            let off = engine::drift(&cat, &st);
            if off.is_empty() {
                println!("{}", tr("Tudo que o ntoskrnl aplicou continua valendo.", "Everything ntoskrnl applied is still in place."));
                return Ok(());
            }
            println!("{}", tr("O Windows desfez:", "Windows undid:"));
            for id in &off {
                println!("  {id}  {}", cat.tweak(id).map_or("", |t| t.title.get()));
            }
            if fix {
                let s = sys::sysinfo();
                report(&engine::apply(&cat, &mut st, &s, &off, &mut print_log))?;
            }
        }
    }
    Ok(())
}
