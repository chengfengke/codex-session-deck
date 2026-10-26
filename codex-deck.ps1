param([Parameter(ValueFromRemainingArguments=$true)][string[]]$DeckArgs)
$ErrorActionPreference = 'Stop'
$DeckArch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
    'Arm64' { 'arm64' }
    'X64' { 'x64' }
    default { throw 'Supported Windows architectures: x64 and arm64.' }
}
$DeckBinary = Join-Path $PSScriptRoot "bin/codex-deck-windows-$DeckArch.exe"
if (-not (Test-Path $DeckBinary)) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'Install stable Rust (MSVC toolchain) and Visual Studio C++ Build Tools, then run scripts/build.ps1.'
    }
    & (Join-Path $PSScriptRoot 'scripts/build.ps1')
}
if (-not $env:CODEX_DECK_CODEX -and -not $env:SESSIONDESK_CODEX) {
    $DeckCodex = Get-Command codex.exe, codex.cmd, codex -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($DeckCodex) { $env:CODEX_DECK_CODEX = $DeckCodex.Source }
}
& $DeckBinary @DeckArgs
exit $LASTEXITCODE
