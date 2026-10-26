$ErrorActionPreference = 'Stop'
$DeckRoot = Split-Path -Parent $PSScriptRoot
$DeckArch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
    'Arm64' { 'arm64' }
    'X64' { 'x64' }
    default { throw 'Supported Windows architectures: x64 and arm64.' }
}
& cargo build --release --locked --manifest-path (Join-Path $DeckRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed.' }
$DeckOutput = Join-Path $DeckRoot 'bin'
New-Item -ItemType Directory -Force -Path $DeckOutput | Out-Null
Copy-Item (Join-Path $DeckRoot 'target/release/codex-deck.exe') (Join-Path $DeckOutput "codex-deck-windows-$DeckArch.exe") -Force
Write-Output "Built codex-deck-windows-$DeckArch.exe"
