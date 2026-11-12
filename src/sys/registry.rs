//! Registro do Windows pela API nativa (winreg). Fora do Windows tudo vira um registro em memória,
//! para desenvolver a interface no Linux sem mexer em nada.

use crate::catalog::RegValue;
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;
pub const REG_DWORD: u32 = 4;
pub const REG_QWORD: u32 = 11;

/// Valor como estava antes de mudar: tipo e bytes crus, assim volta exatamente igual
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Snapshot {
    pub path: String,
    pub name: String,
    pub exists: bool,
    #[serde(default)]
    pub vtype: u32,
    #[serde(default)]
    pub hex: String,
}

impl Snapshot {
    pub fn same_key(&self, path: &str, name: &str) -> bool {
        self.path.eq_ignore_ascii_case(path) && self.name.eq_ignore_ascii_case(name)
    }

    fn bytes(&self) -> Vec<u8> {
        (0..self.hex.len() / 2).filter_map(|i| u8::from_str_radix(&self.hex[i * 2..i * 2 + 2], 16).ok()).collect()
    }

    /// Valor legível (número ou texto); binários ficam em hex
    pub fn display(&self) -> String {
        if !self.exists {
            return crate::catalog::tr("(não existe)", "(missing)").into();
        }
        let b = self.bytes();
        match self.vtype {
            REG_DWORD if b.len() >= 4 => {
                let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                if n > 9 { format!("{n} (0x{n:X})") } else { n.to_string() }
            }
            REG_QWORD if b.len() >= 8 => u64::from_le_bytes(b[..8].try_into().unwrap()).to_string(),
            REG_SZ | REG_EXPAND_SZ => format!("\"{}\"", utf16(&b)),
            _ => format!("hex:{}", self.hex),
        }
    }

    /// O valor atual é o que o tweak grava. DWORD/QWORD comparam como número; texto compara como texto.
    pub fn matches(&self, want: &RegValue) -> bool {
        if !self.exists {
            return false;
        }
        let b = self.bytes();
        let num = match self.vtype {
            REG_DWORD if b.len() >= 4 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64),
            REG_QWORD if b.len() >= 8 => Some(u64::from_le_bytes(b[..8].try_into().unwrap())),
            _ => None,
        };
        match (want, num) {
            (RegValue::Num(w), Some(n)) => *w as u64 == n,
            (RegValue::Num(w), None) => utf16(&b).trim() == w.to_string(),
            (RegValue::Str(w), Some(n)) => w.trim() == n.to_string(),
            (RegValue::Str(w), None) => utf16(&b) == *w,
        }
    }
}

fn utf16(b: &[u8]) -> String {
    let w: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    String::from_utf16_lossy(&w).trim_end_matches('\0').to_string()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::{bail, Context};
    use winreg::enums::{RegType, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_ALL_ACCESS, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY};
    use winreg::RegKey;

    fn split(path: &str) -> Result<(RegKey, &str)> {
        let (root, sub) = path.split_once('\\').unwrap_or((path, ""));
        let hk = match root.to_ascii_uppercase().as_str() {
            "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
            "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
            "HKCR" | "HKEY_CLASSES_ROOT" => HKEY_CLASSES_ROOT,
            "HKU" | "HKEY_USERS" => HKEY_USERS,
            _ => bail!("raiz de registro desconhecida: {root}"),
        };
        Ok((RegKey::predef(hk), sub))
    }

    fn vtype_of(n: u32) -> RegType {
        use winreg::enums::RegType as T;
        match n {
            0 => T::REG_NONE,
            1 => T::REG_SZ,
            2 => T::REG_EXPAND_SZ,
            4 => T::REG_DWORD,
            5 => T::REG_DWORD_BIG_ENDIAN,
            6 => T::REG_LINK,
            7 => T::REG_MULTI_SZ,
            11 => T::REG_QWORD,
            _ => T::REG_BINARY,
        }
    }

    pub fn read(path: &str, name: &str) -> Snapshot {
        let mut s = Snapshot { path: path.into(), name: name.into(), exists: false, vtype: 0, hex: String::new() };
        if let Ok((root, sub)) = split(path) {
            if let Ok(k) = root.open_subkey_with_flags(sub, KEY_READ | KEY_WOW64_64KEY) {
                if let Ok(v) = k.get_raw_value(name) {
                    s.exists = true;
                    s.vtype = v.vtype as u32;
                    s.hex = to_hex(&v.bytes);
                }
            }
        }
        s
    }

    pub fn set(path: &str, name: &str, kind: &str, value: &RegValue) -> Result<()> {
        let (root, sub) = split(path)?;
        let (k, _) = root
            .create_subkey_with_flags(sub, KEY_ALL_ACCESS | KEY_WOW64_64KEY)
            .with_context(|| format!("abrindo {path}"))?;
        let r = match (kind, value) {
            ("REG_DWORD", RegValue::Num(n)) => k.set_value(name, n),
            ("REG_DWORD", RegValue::Str(s)) => k.set_value(name, &s.parse::<u32>().context("valor DWORD inválido")?),
            ("REG_SZ", RegValue::Str(s)) => k.set_value(name, s),
            ("REG_SZ", RegValue::Num(n)) => k.set_value(name, &n.to_string()),
            _ => bail!("tipo de registro não suportado: {kind}"),
        };
        r.with_context(|| format!("gravando {path}\\{name}"))
    }

    pub fn restore(s: &Snapshot) -> Result<()> {
        let (root, sub) = split(&s.path)?;
        if !s.exists {
            // não existia antes: apaga (se a chave nem existe, não há o que apagar)
            if let Ok(k) = root.open_subkey_with_flags(sub, KEY_SET_VALUE | KEY_WOW64_64KEY) {
                match k.delete_value(&s.name) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
            return Ok(());
        }
        let (k, _) = root.create_subkey_with_flags(sub, KEY_ALL_ACCESS | KEY_WOW64_64KEY)?;
        let v = winreg::RegValue { bytes: s.bytes(), vtype: vtype_of(s.vtype) };
        k.set_raw_value(&s.name, &v).with_context(|| format!("restaurando {}\\{}", s.path, s.name))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    static MEM: Mutex<Option<HashMap<String, (u32, String)>>> = Mutex::new(None);

    fn key(path: &str, name: &str) -> String {
        format!("{}\\{}", path, name).to_ascii_lowercase()
    }

    pub fn read(path: &str, name: &str) -> Snapshot {
        let m = MEM.lock().unwrap();
        let v = m.as_ref().and_then(|m| m.get(&key(path, name)).cloned());
        Snapshot { path: path.into(), name: name.into(), exists: v.is_some(), vtype: v.as_ref().map_or(0, |x| x.0), hex: v.map(|x| x.1).unwrap_or_default() }
    }

    pub fn set(path: &str, name: &str, _kind: &str, value: &RegValue) -> Result<()> {
        let v = match value {
            RegValue::Num(n) => (REG_DWORD, to_hex(&n.to_le_bytes())),
            RegValue::Str(s) => (REG_SZ, to_hex(&s.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect::<Vec<_>>())),
        };
        MEM.lock().unwrap().get_or_insert_with(HashMap::new).insert(key(path, name), v);
        Ok(())
    }

    pub fn restore(s: &Snapshot) -> Result<()> {
        let mut m = MEM.lock().unwrap();
        let m = m.get_or_insert_with(HashMap::new);
        if s.exists {
            m.insert(key(&s.path, &s.name), (s.vtype, s.hex.clone()));
        } else {
            m.remove(&key(&s.path, &s.name));
        }
        Ok(())
    }
}

pub use imp::{read, restore, set};
