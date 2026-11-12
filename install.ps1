# ntoskrnl: baixa a última versão do GitHub, confere o SHA256 e abre.
#   irm https://github.com/grimh3xx/ntoskrnl/releases/latest/download/install.ps1 | iex
# Com argumentos (sem abrir a TUI):
#   & ([scriptblock]::Create((irm https://github.com/grimh3xx/ntoskrnl/releases/latest/download/install.ps1))) profile gamer

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$base = 'https://github.com/grimh3xx/ntoskrnl/releases/latest/download'
$dir = Join-Path $env:TEMP 'ntoskrnl'
$exe = Join-Path $dir 'ntoskrnl.exe'
New-Item -ItemType Directory -Force -Path $dir | Out-Null

Write-Host 'ntoskrnl: baixando...' -ForegroundColor Cyan
Invoke-WebRequest "$base/ntoskrnl.exe" -OutFile $exe -UseBasicParsing
# no Windows PowerShell 5.1 o .sha256 (application/octet-stream) chega como byte[]
$raw = (Invoke-WebRequest "$base/ntoskrnl.exe.sha256" -UseBasicParsing).Content
if ($raw -is [byte[]]) { $raw = [Text.Encoding]::ASCII.GetString($raw) }
$expected = ($raw.Trim() -split '\s+')[0].ToLower()
$actual = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
if ($expected -ne $actual) {
    Remove-Item $exe -Force
    throw "SHA256 não confere (esperado $expected, veio $actual). Nada foi executado."
}
Write-Host "ntoskrnl: SHA256 ok ($actual)" -ForegroundColor DarkGray

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($admin) {
    & $exe @args
} else {
    # o exe pede administrador; elevado, ele abre numa janela nova
    if ($args.Count) { Start-Process $exe -ArgumentList $args -Verb RunAs } else { Start-Process $exe -Verb RunAs }
}
