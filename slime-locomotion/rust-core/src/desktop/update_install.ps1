param([Parameter(Mandatory=$true)][string]$Request, [switch]$NoRestart)
$ErrorActionPreference = 'Stop'
$r = Get-Content -LiteralPath $Request -Raw | ConvertFrom-Json
$stage = [IO.Path]::GetFullPath((Split-Path -Parent $Request))
$target = [IO.Path]::GetFullPath([string]$r.target)
$candidate = Join-Path $stage 'candidate.exe'
$backup = Join-Path $stage 'previous.exe'
$result = Join-Path $stage 'result.json'
$moved = $false
$installed = $false
try {
    $expectedRoot = Join-Path (Split-Path -Parent $target) '.arcane-update'
    if ((Split-Path -Parent $stage) -ine $expectedRoot -or [IO.Path]::GetExtension($target) -ine '.exe') { throw 'Invalid update destination' }
    # Preserve relative --config paths. Older staged requests retain their original fallback.
    $restartDirectory = Split-Path -Parent $target
    if ($r.restart_working_directory) { $restartDirectory = [IO.Path]::GetFullPath([string]$r.restart_working_directory) }
    # If the original folder is gone, still install the update but do not restart: launching
    # from another folder could resolve a relative --config path to the wrong settings file.
    $restart = !$NoRestart
    $note = $null
    if ($restart -and !(Test-Path -LiteralPath $restartDirectory -PathType Container)) {
        $restart = $false
        $note = 'Updated; not restarted because the original working directory is no longer available. Start ArcaneVR again.'
    }
    if (Test-Path -LiteralPath $backup) { throw 'Recovery file already exists' }
    if ($r.sha256 -notmatch '^[0-9a-fA-F]{64}$') { throw 'Invalid expected checksum' }
    if ((Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash -ine $r.sha256) { throw 'Downloaded file checksum mismatch' }
    $bytes = [IO.File]::ReadAllBytes($candidate)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 77 -or $bytes[1] -ne 90) { throw 'Downloaded file is not a Windows executable' }
    if ([int]$r.parent_pid -gt 0) {
        $owner = Get-Process -Id ([int]$r.parent_pid) -ErrorAction SilentlyContinue
        if ($owner) {
            if ($owner.StartTime.ToUniversalTime().Ticks -ne [long]$r.parent_start_ticks) { throw 'Parent process identity changed' }
            Wait-Process -Id $owner.Id -Timeout 60
        }
    }
    if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ine $r.original_sha256) { throw 'Installed executable changed while update was staged' }
    Move-Item -LiteralPath $target -Destination $backup
    $moved = $true
    Move-Item -LiteralPath $candidate -Destination $target
    $installed = $true
    if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ine $r.sha256) { throw 'Installed file checksum mismatch' }
    if ($restart) {
        # Arguments are written by ArcaneVR, never taken from release metadata.
        # Windows quoting escapes trailing backslashes and embedded quotes.
        $quoted = @($r.restart_args | ForEach-Object {
            $arg = [string]$_
            '"' + ([regex]::Replace([regex]::Replace($arg, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1')) + '"'
        })
        $launch = @{FilePath=$target; WorkingDirectory=$restartDirectory; WindowStyle='Normal'; PassThru=$true}
        if ($quoted.Count -gt 0) { $launch.ArgumentList = ($quoted -join ' ') }
        Start-Process @launch | Out-Null
    }
    @{status='installed'; target=$target; backup=$backup; sha256=$r.sha256; note=$note} | ConvertTo-Json | Set-Content -LiteralPath $result -Encoding UTF8
} catch {
    $message = $_.Exception.Message
    if ($moved) {
        try {
            if ($installed -and (Test-Path -LiteralPath $target)) {
                Move-Item -LiteralPath $target -Destination (Join-Path $stage 'failed.exe')
            }
            Move-Item -LiteralPath $backup -Destination $target
        } catch { $message += '; recovery failed: ' + $_.Exception.Message }
    }
    @{status='failed'; error=$message; target=$target; backup=$backup} | ConvertTo-Json | Set-Content -LiteralPath $result -Encoding UTF8
    exit 1
}
