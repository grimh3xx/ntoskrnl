//! Processos externos: PowerShell (scripts dos tweaks, AppX, ponto de restauração) e programas do Windows
//! (winget, sfc, DISM). Sem janela, com tempo máximo e a saída lida em paralelo para não travar o pipe.

use anyhow::{bail, Result};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Saída UTF-8, sem barra de progresso (ela polui o stdout) e erro não interrompe por padrão
const PRELUDE: &str = "[Console]::OutputEncoding = [Text.Encoding]::UTF8\n$ProgressPreference = 'SilentlyContinue'\n";

/// Roda um script PowerShell. Vai em -EncodedCommand (UTF-16LE em base64): sem problema de aspas.
pub fn ps(script: &str, env: &[(&str, &str)], timeout: Duration) -> Result<Output> {
    let full = format!("{PRELUDE}{script}");
    let utf16: Vec<u8> = full.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    let args = ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &base64(&utf16)];
    run("powershell.exe", &args, env, timeout, |_| {})
}

/// Roda um script e devolve o stdout como JSON
pub fn ps_json<T: serde::de::DeserializeOwned>(script: &str, timeout: Duration) -> Result<T> {
    let o = ps(script, &[], timeout)?;
    if o.code != 0 {
        bail!("{}", first_line(&o.stderr));
    }
    Ok(serde_json::from_str(o.stdout.trim().trim_start_matches('\u{feff}'))?)
}

/// Roda um programa. `on_line` recebe cada linha do stdout assim que sai (log ao vivo na TUI).
pub fn run(exe: &str, args: &[&str], env: &[(&str, &str)], timeout: Duration, mut on_line: impl FnMut(&str)) -> Result<Output> {
    if !cfg!(windows) {
        return Ok(mock(exe, args));
    }
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("não consegui rodar {exe}: {e}"))?;
    let mut err = child.stderr.take().unwrap();
    let err_t = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        String::from_utf8_lossy(&b).into_owned()
    });
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let out = child.stdout.take().unwrap();
    let out_t = thread::spawn(move || {
        // winget redesenha a barra de progresso com \r: cada pedaço entre \r ou \n vira uma "linha"
        let mut r = BufReader::new(out);
        let mut buf = Vec::new();
        while r.read_until(b'\n', &mut buf).map(|n| n > 0).unwrap_or(false) {
            let s = decode(&buf);
            for part in s.split('\r') {
                let _ = tx.send(part.trim_end_matches('\n').to_string());
            }
            buf.clear();
        }
    });

    let start = Instant::now();
    let mut stdout = String::new();
    let code = loop {
        while let Ok(l) = rx.try_recv() {
            if !l.trim().is_empty() {
                on_line(&l);
            }
            stdout.push_str(&l);
            stdout.push('\n');
        }
        if let Some(st) = child.try_wait()? {
            break st.code().unwrap_or(-1);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            bail!("{exe} passou do tempo máximo ({}s)", timeout.as_secs());
        }
        thread::sleep(Duration::from_millis(40));
    };
    let _ = out_t.join();
    for l in rx.try_iter() {
        if !l.trim().is_empty() {
            on_line(&l);
        }
        stdout.push_str(&l);
        stdout.push('\n');
    }
    Ok(Output { code, stdout, stderr: err_t.join().unwrap_or_default() })
}

/// sfc e DISM às vezes escrevem em UTF-16 quando a saída é redirecionada
fn decode(b: &[u8]) -> String {
    if b.len() >= 2 && b.iter().skip(1).step_by(2).filter(|&&x| x == 0).count() * 2 >= b.len() / 2 {
        let w: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        return String::from_utf16_lossy(&w).replace('\0', "");
    }
    String::from_utf8_lossy(b).into_owned()
}

pub fn first_line(s: &str) -> String {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(200).collect()
}

/// Fora do Windows: finge que deu certo, para testar a interface
fn mock(exe: &str, _args: &[&str]) -> Output {
    thread::sleep(Duration::from_millis(150));
    let stdout = if exe == "powershell.exe" { "ok\n".into() } else { format!("[simulado] {exe}\n") };
    Output { code: 0, stdout, stderr: String::new() }
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            o.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    o
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
