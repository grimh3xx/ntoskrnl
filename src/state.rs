//! Backups dos tweaks aplicados (state.json). É o que permite desfazer: o original é gravado ANTES de mudar.

use crate::sys::{data_dir, registry::Snapshot};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Backup {
    /// segundos desde 1970
    pub applied_at: u64,
    pub registry: Vec<Snapshot>,
    /// saída do script de backup de cada ação de comando, por chave
    pub cmd: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct State {
    pub backups: BTreeMap<String, Backup>,
    /// versão do Windows (build.UBR) na última conferência: detecta atualização que desfez tweaks
    pub windows_build: Option<String>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl State {
    pub fn load() -> Self {
        let p = data_dir().join("state.json");
        std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Grava num arquivo temporário e troca: um crash no meio nunca deixa o state.json pela metade
    pub fn save(&self) -> Result<()> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("criando {}", dir.display()))?;
        let tmp = dir.join("state.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, dir.join("state.json"))?;
        Ok(())
    }
}

/// Data UTC curta (AAAA-MM-DD HH:MM) sem depender de crate de data
pub fn fmt_time(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let (h, m) = ((secs % 86400) / 3600, (secs % 3600) / 60);
    // algoritmo civil_from_days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02} UTC")
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_dates() {
        assert_eq!(super::fmt_time(0), "1970-01-01 00:00 UTC");
        assert_eq!(super::fmt_time(1_793_000_000), "2026-10-26 07:33 UTC");
    }
}
