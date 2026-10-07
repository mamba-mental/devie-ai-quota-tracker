param([switch]$SkipChecks)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $projectRoot

if ($env:OS -ne 'Windows_NT') {
    throw 'Run this build script on Windows.'
}

foreach ($command in @('bun', 'cargo')) {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
        throw "Required build tool is missing: $command"
    }
}

$toolDirectories = @('bun', 'cargo') | ForEach-Object { Split-Path -Parent (Get-Command $_).Source }
$env:Path = ($toolDirectories -join ';') + ';' + $env:Path

function Invoke-BuildStep([string]$Name, [scriptblock]$Action) {
    Write-Host "`n$Name"
    $ErrorActionPreference = 'Continue'
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed (exit $LASTEXITCODE)."
    }
}

Invoke-BuildStep 'Install locked dependencies' { bun install --frozen-lockfile }
if (-not $SkipChecks) {
    Invoke-BuildStep 'Check interface code' { bun run check }
    Invoke-BuildStep 'Check translations' { bun run i18n:verify }
    Invoke-BuildStep 'Build desktop interface' { bun run build:desktop-frontend }
    Invoke-BuildStep 'Check Rust formatting' { cargo fmt --check --manifest-path src-desktop/Cargo.toml }
    Invoke-BuildStep 'Test Windows application core' { cargo test --locked --manifest-path src-desktop/Cargo.toml }
}
Invoke-BuildStep 'Build Windows installer' { bun run tauri build --bundles nsis -- --locked }
Get-ChildItem -LiteralPath 'src-desktop\target\release\bundle\nsis' -Filter '*.exe' |
    Select-Object FullName, Length, LastWriteTime
