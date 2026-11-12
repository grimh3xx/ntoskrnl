//! Catálogo embutido no exe (pasta catalog/). Funciona offline; `--catalog <pasta>` troca por arquivos locais.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Pt,
    En,
}

static LANG: OnceLock<Lang> = OnceLock::new();

pub fn set_lang(l: Lang) {
    let _ = LANG.set(l);
}

pub fn lang() -> Lang {
    *LANG.get().unwrap_or(&Lang::Pt)
}

/// Texto nos dois idiomas
#[derive(Deserialize, Clone, Debug, Default)]
pub struct L10n {
    pub pt: String,
    pub en: String,
}

impl L10n {
    pub fn get(&self) -> &str {
        match lang() {
            Lang::Pt => &self.pt,
            Lang::En => &self.en,
        }
    }
}

/// Atalho para textos da interface
pub fn tr<'a>(pt: &'a str, en: &'a str) -> &'a str {
    match lang() {
        Lang::Pt => pt,
        Lang::En => en,
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Safe,
    Moderate,
    Advanced,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Safe => tr("seguro", "safe"),
            Risk::Moderate => tr("moderado", "moderate"),
            Risk::Advanced => tr("avançado", "advanced"),
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum RegValue {
    Num(u32),
    Str(String),
}

#[derive(Deserialize, Clone, Debug)]
pub struct RegAction {
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: RegValue,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StartMode {
    Auto,
    Manual,
    Disabled,
}

impl StartMode {
    pub fn dword(self) -> u32 {
        match self {
            StartMode::Auto => 2,
            StartMode::Manual => 3,
            StartMode::Disabled => 4,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct ServiceAction {
    pub name: String,
    pub start: StartMode,
}

/// Script PowerShell. `backup` imprime um texto que volta em $env:NTOSKRNL_BACKUP no `apply`, `verify` e `revert`.
#[derive(Deserialize, Clone, Debug)]
pub struct CmdAction {
    pub key: String,
    pub apply: String,
    pub revert: String,
    pub backup: Option<String>,
    /// imprime "ok" se o Windows aceitou, ou o motivo se não aceitou
    pub verify: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Requires {
    pub min_build: Option<u32>,
    pub gpu: Option<String>,
    #[serde(default)]
    pub ssd: bool,
    pub min_ram_gb: Option<f64>,
    #[serde(default)]
    pub desktop_only: bool,
    #[serde(default)]
    pub not_home: bool,
    #[serde(default)]
    pub home_only: bool,
    #[serde(default)]
    pub defender: bool,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Tweak {
    pub id: String,
    pub category: String,
    pub risk: Risk,
    #[serde(default)]
    pub restart: bool,
    pub title: L10n,
    pub summary: Option<L10n>,
    pub description: L10n,
    pub warning: Option<L10n>,
    pub requires: Option<Requires>,
    #[serde(default)]
    pub reg: Vec<RegAction>,
    #[serde(default)]
    pub service: Vec<ServiceAction>,
    #[serde(default)]
    pub cmd: Vec<CmdAction>,
}

impl Tweak {
    /// Serviços viram o valor Start no registro: backup e reversão usam o mesmo mecanismo
    pub fn reg_actions(&self) -> Vec<RegAction> {
        let mut v = self.reg.clone();
        v.extend(self.service.iter().map(|s| RegAction {
            path: format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{}", s.name),
            name: "Start".into(),
            kind: "REG_DWORD".into(),
            value: RegValue::Num(s.start.dword()),
        }));
        v
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct App {
    pub id: String,
    pub name: String,
    pub category: String,
    pub desc: L10n,
    pub source: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Bloat {
    pub id: String,
    pub name: String,
    pub desc: L10n,
    pub caution: Option<L10n>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Profile {
    pub id: String,
    pub title: L10n,
    pub description: L10n,
    pub tweaks: Vec<String>,
    #[serde(default)]
    pub debloat: Vec<String>,
}

#[derive(Deserialize)]
struct Tweaks {
    tweak: Vec<Tweak>,
}
#[derive(Deserialize)]
struct Apps {
    app: Vec<App>,
}
#[derive(Deserialize)]
struct Debloat {
    bloat: Vec<Bloat>,
}
#[derive(Deserialize)]
struct Profiles {
    profile: Vec<Profile>,
}

pub struct Catalog {
    pub tweaks: Vec<Tweak>,
    pub apps: Vec<App>,
    pub bloat: Vec<Bloat>,
    pub profiles: Vec<Profile>,
}

const EMBEDDED: [(&str, &str); 4] = [
    ("tweaks.toml", include_str!("../catalog/tweaks.toml")),
    ("apps.toml", include_str!("../catalog/apps.toml")),
    ("debloat.toml", include_str!("../catalog/debloat.toml")),
    ("profiles.toml", include_str!("../catalog/profiles.toml")),
];

impl Catalog {
    pub fn load(dir: Option<&Path>) -> Result<Self> {
        let text = |i: usize| -> Result<String> {
            let (name, embedded) = EMBEDDED[i];
            match dir {
                Some(d) => std::fs::read_to_string(d.join(name)).with_context(|| format!("lendo {}", d.join(name).display())),
                None => Ok(embedded.to_string()),
            }
        };
        let tweaks: Tweaks = toml::from_str(&text(0)?).context("tweaks.toml")?;
        let apps: Apps = toml::from_str(&text(1)?).context("apps.toml")?;
        let bloat: Debloat = toml::from_str(&text(2)?).context("debloat.toml")?;
        let profiles: Profiles = toml::from_str(&text(3)?).context("profiles.toml")?;
        let c = Catalog { tweaks: tweaks.tweak, apps: apps.app, bloat: bloat.bloat, profiles: profiles.profile };
        c.check()?;
        Ok(c)
    }

    /// Perfis só podem citar ids que existem
    fn check(&self) -> Result<()> {
        for p in &self.profiles {
            for id in &p.tweaks {
                anyhow::ensure!(self.tweak(id).is_some(), "perfil {}: tweak desconhecido {id}", p.id);
            }
            for id in &p.debloat {
                anyhow::ensure!(self.bloat.iter().any(|b| &b.id == id), "perfil {}: app desconhecido {id}", p.id);
            }
        }
        Ok(())
    }

    pub fn tweak(&self, id: &str) -> Option<&Tweak> {
        self.tweaks.iter().find(|t| t.id == id)
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }
}

pub fn category_label(c: &str) -> &'static str {
    match c {
        "power" => tr("Energia", "Power"),
        "gaming" => tr("Jogos", "Gaming"),
        "gpu" => tr("Placa de vídeo", "GPU"),
        "input" => tr("Mouse e teclado", "Input"),
        "system" => tr("Sistema", "System"),
        "network" => tr("Rede", "Network"),
        "privacy" => tr("Privacidade", "Privacy"),
        "services" => tr("Serviços", "Services"),
        "security" => tr("Segurança", "Security"),
        "games" => tr("Jogos e launchers", "Games and launchers"),
        "pc" => tr("Ferramentas de PC", "PC tools"),
        "chat" => tr("Conversa", "Chat"),
        "browser" => tr("Navegadores", "Browsers"),
        "creator" => tr("Criação", "Creator"),
        "media" => tr("Mídia", "Media"),
        "tools" => tr("Utilitários", "Utilities"),
        "runtime" => tr("Runtimes", "Runtimes"),
        _ => "?",
    }
}
