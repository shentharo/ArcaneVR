# Builds ArcaneVR 1.1.21 from this source tree and runs its tests.
# It does NOT install anything and does not touch the running app or its settings.
# Written without access to a Windows machine: read it before running it.
$ErrorActionPreference = 'Stop'
$src  = $PSScriptRoot
$room = Join-Path $src 'slime-locomotion\bevy-test-room'
$core = Join-Path $src 'slime-locomotion\rust-core'
function Run([string]$what, [scriptblock]$command) {
    Write-Host "== $what"
    & $command
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit $LASTEXITCODE)" }
}
Set-Location $room
Run 'Renderer tests' { cargo +1.95.0 test --release }
Run 'Renderer build' { cargo +1.95.0 build --release }
$renderer = Join-Path $room 'target\release\arcane-test-room.exe'
if (-not (Test-Path $renderer)) { throw "Renderer not found at $renderer (is CARGO_TARGET_DIR set?)" }
$env:ARCANEVR_EMBED_RENDERER = $renderer
Set-Location $core
Run 'Core tests'    { cargo +1.95.0 test --release --lib }
Run 'Desktop tests' { cargo +1.95.0 test --release --bin vrcraft }
Run 'Release build' { cargo +1.95.0 build --release --bin vrcraft --bin replay-steps }
$exe = Join-Path $core 'target\release\vrcraft.exe'
if (-not (Test-Path $exe)) { throw "Build output not found at $exe" }
Write-Host ''
Write-Host "Built: $exe"
Write-Host ("SHA-256: " + (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash)
Write-Host 'Not installed. Run the receiver QA in AGENTS.md against this exe before installing it.'
