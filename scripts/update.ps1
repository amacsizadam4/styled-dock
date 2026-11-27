# Embedded in the executable. No credentials or administrator privileges required.
param(
    [Parameter(Mandatory)][string]$CurrentVersion,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$WorkingDirectory,
    [Parameter(Mandatory)][string]$Root,
    [switch]$Install
)
$ErrorActionPreference = 'Stop'
$mutex = New-Object Threading.Mutex($false, 'Local\StyledDock.Update')
if (-not $mutex.WaitOne(0)) { $mutex.Dispose(); exit 0 }
function Status([string]$Message) { [IO.File]::WriteAllText((Join-Path $Root 'status.txt'), $Message) }
function StableVersion([string]$Tag) {
    if ($Tag -cnotmatch '^v?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw 'Invalid stable version' }
    return [version]($Tag.TrimStart('v'))
}
function SelectAsset($release) {
    $assets = @($release.assets | Where-Object { $_.name -ceq 'styled-dock.exe' })
    if ($assets.Count -ne 1) { throw 'Release must contain styled-dock.exe' }
    $asset = $assets[0]
    if ($asset.digest -cnotmatch '^sha256:[a-fA-F0-9]{64}$') { throw 'Release asset has no SHA-256 digest' }
    if ($asset.browser_download_url -notlike 'https://github.com/amacsizadam4/styled-dock/releases/download/*') { throw 'Unexpected download URL' }
    return $asset
}
function CheckBinary([byte[]]$binary) {
    if ($binary.Length -lt 64 -or $binary[0] -ne 77 -or $binary[1] -ne 90) { throw 'Invalid Windows executable' }
    $offset = [BitConverter]::ToInt32($binary, 60)
    if ($offset -lt 64 -or $offset -gt $binary.Length - 6) { throw 'Invalid PE offset' }
    if ([BitConverter]::ToUInt32($binary, $offset) -ne 17744 -or [BitConverter]::ToUInt16($binary, $offset + 4) -ne 34404) { throw 'Expected Windows x64 executable' }
}
$backup = "$Executable.update-backup"
$replaced = $false
$stopped = $false
try {
    Status 'Checking for updates...'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $headers = @{ 'User-Agent' = "StyledDock/$CurrentVersion"; Accept = 'application/vnd.github+json' }
    $release = Invoke-RestMethod -Uri 'https://api.github.com/repos/amacsizadam4/styled-dock/releases/latest' -Headers $headers -TimeoutSec 30
    if ($release.draft -or $release.prerelease) { throw 'Expected a stable public release' }
    if ((StableVersion $release.tag_name) -le (StableVersion $CurrentVersion)) {
        Status 'Styled Dock is up to date.'; exit 0
    }
    $asset = SelectAsset $release
    if (-not $Install) { Status 'An update is available. Choose Update and restart.'; exit 0 }
    # Verify write access before interrupting the dock.
    $stage = Join-Path ([IO.Path]::GetDirectoryName($Executable)) ('.styled-update-' + [guid]::NewGuid().ToString('N') + '.exe')
    $probe = [IO.File]::Create($stage); $probe.Dispose()
    Status 'Downloading update...'
    Invoke-WebRequest -UseBasicParsing -Uri $asset.browser_download_url -OutFile $stage -TimeoutSec 180
    if ((Get-Item -LiteralPath $stage).Length -ne $asset.size) { throw 'Download size mismatch' }
    if ((Get-FileHash -LiteralPath $stage -Algorithm SHA256).Hash -ine $asset.digest.Substring(7)) { throw 'Download checksum mismatch' }
    $binary = [IO.File]::ReadAllBytes($stage)
    CheckBinary $binary
    $productVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($stage).ProductVersion
    if ((StableVersion $productVersion) -ne (StableVersion $release.tag_name)) { throw 'Executable version does not match release tag' }
    Status 'Installing update and restarting...'
    # --quit uses the existing graceful shutdown/taskbar restoration path.
    $quit = Start-Process -FilePath $Executable -ArgumentList '--quit' -WorkingDirectory $WorkingDirectory -PassThru
    if (-not $quit.WaitForExit(15000) -or $quit.ExitCode -ne 0) { throw 'Could not stop Styled Dock' }
    $stopped = $true
    # The watchdog also runs from this executable; wait for it to finish restoring Explorer.
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $running = @(Get-Process -Name 'styled-dock' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Executable })
        if ($running.Count -eq 0) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($running.Count -ne 0) { throw 'Styled Dock processes are still running' }
    [IO.File]::Replace($stage, $Executable, $backup)
    $replaced = $true
    $ready = Join-Path $Root ([guid]::NewGuid().ToString('N') + '.ready')
    $env:STYLED_DOCK_UPDATE_READY = $ready
    try { $new = Start-Process -FilePath $Executable -WorkingDirectory $WorkingDirectory -PassThru }
    finally { Remove-Item Env:\STYLED_DOCK_UPDATE_READY }
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path -LiteralPath $ready)) {
        if ($new.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw 'Updated application did not initialize' }
        Start-Sleep -Milliseconds 200
    }
    if ($new.WaitForExit(2000)) { throw 'Updated application exited during startup' }
    Status 'Update installed. Previous executable kept as .update-backup.'
} catch {
    $failure = $_.Exception.Message
    try {
        if ($replaced) {
            if ($new -and -not $new.HasExited) {
                $stop = Start-Process -FilePath $Executable -ArgumentList '--quit' -WorkingDirectory $WorkingDirectory -PassThru
                if (-not $stop.WaitForExit(15000) -or -not $new.WaitForExit(15000)) { throw 'Updated application is still running; backup retained' }
                Start-Sleep -Seconds 2 # Allow its watchdog to finish restoration.
            }
            [IO.File]::Copy($backup, $Executable, $true)
        }
        if ($stopped) { Start-Process -FilePath $Executable -WorkingDirectory $WorkingDirectory | Out-Null }
    } catch { $failure += "; Recovery: $($_.Exception.Message)" }
    [IO.File]::WriteAllText((Join-Path $Root 'error.log'), $failure)
    Status 'Update failed. See Updates/error.log in the StyledDock data folder.'
    exit 1
} finally {
    if ($stage -and (Test-Path -LiteralPath $stage)) { Remove-Item -LiteralPath $stage -Force }
    if ($ready -and (Test-Path -LiteralPath $ready)) { Remove-Item -LiteralPath $ready -Force }
    $mutex.ReleaseMutex(); $mutex.Dispose()
}
