pub mod proc;
pub mod registry;

use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

/// O que os requisitos dos tweaks precisam saber do PC. Uma consulta só, na abertura.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct SysInfo {
    pub build: u32,
    pub ubr: u32,
    pub edition: String,
    pub display: String,
    pub cpu: String,
    pub gpus: String,
    pub ram_gb: f64,
    pub laptop: bool,
    pub ssd: bool,
    pub defender: bool,
}

impl SysInfo {
    pub fn gpu_vendor(&self) -> &'static str {
        let g = self.gpus.to_ascii_lowercase();
        if g.contains("nvidia") || g.contains("geforce") {
            "nvidia"
        } else if g.contains("amd") || g.contains("radeon") {
            "amd"
        } else if g.contains("intel") {
            "intel"
        } else {
            "?"
        }
    }

    /// EditionID "Core*" = Home
    pub fn is_home(&self) -> bool {
        self.edition.starts_with("Core")
    }

    pub fn os_name(&self) -> String {
        let v = if self.build >= 22000 { "Windows 11" } else { "Windows 10" };
        format!("{v} {} ({}.{})", self.display, self.build, self.ubr)
    }
}

const INFO_PS: &str = r#"
$cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$ssd = $false
try {
  $n = (Get-Partition -DriveLetter $env:SystemDrive[0] -ErrorAction Stop).DiskNumber
  $ssd = [string](Get-PhysicalDisk -ErrorAction Stop | Where-Object DeviceId -eq "$n").MediaType -eq 'SSD'
} catch {}
$def = $false
try { $m = Get-MpComputerStatus -ErrorAction Stop; $def = [bool]($m.AntivirusEnabled -and $m.AMServiceEnabled) } catch {}
[pscustomobject]@{
  build = [int]$cv.CurrentBuild
  ubr = [int]$cv.UBR
  edition = [string]$cv.EditionID
  display = [string]$cv.DisplayVersion
  cpu = [string](Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
  gpus = ((Get-CimInstance Win32_VideoController).Name -join '; ')
  ram_gb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
  laptop = [bool](Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue)
  ssd = $ssd
  defender = $def
} | ConvertTo-Json -Compress
"#;

pub fn sysinfo() -> SysInfo {
    if !cfg!(windows) {
        return SysInfo {
            build: 26100,
            ubr: 4202,
            edition: "Professional".into(),
            display: "24H2".into(),
            cpu: "AMD Ryzen 7 7800X3D".into(),
            gpus: "NVIDIA GeForce RTX 4070".into(),
            ram_gb: 32.0,
            laptop: false,
            ssd: true,
            defender: true,
        };
    }
    proc::ps_json(INFO_PS, Duration::from_secs(60)).unwrap_or_default()
}

/// %ProgramData%\ntoskrnl: backups e log ficam aqui, fora da pasta do exe (que roda do %TEMP%)
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("NTOSKRNL_DATA") {
        return PathBuf::from(d);
    }
    let base = if cfg!(windows) {
        std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir)
    };
    base.join("ntoskrnl")
}
