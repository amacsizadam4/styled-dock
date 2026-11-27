# Run from regular PowerShell on the Windows desktop used for Styled Dock.
[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateSet('Test', 'Launch', 'Smoke')]
    [string]$Mode,

    [Parameter(Position = 1)]
    [ValidateNotNullOrEmpty()]
    [string]$Filter,

    [switch]$Fullscreen,
    [switch]$Taskbar
)

$ErrorActionPreference = 'Stop'

if (($Mode -eq 'Test') -ne [bool]$Filter) {
    throw 'Test requires a test filter. Launch and Smoke take no filter.'
}
if ($Mode -ne 'Smoke' -and ($Fullscreen -or $Taskbar)) {
    throw '-Fullscreen and -Taskbar are only available in Smoke mode.'
}
if (Get-Process -Name 'styled-dock' -ErrorAction SilentlyContinue) {
    throw 'Quit the running Styled Dock instance before building or testing.'
}

$cargo = (Get-Command cargo -CommandType Application).Source
$devRoot = Join-Path $env:LOCALAPPDATA 'StyledDockDev'
$logDir = Join-Path $devRoot 'logs'
$null = New-Item -ItemType Directory -Force -Path $logDir
$runId = [Guid]::NewGuid().ToString('N').Substring(0, 8)
$logPath = Join-Path $logDir "$(Get-Date -Format yyyyMMdd-HHmmss)-$Mode-$runId.log"

function Invoke-Step {
    param(
        [string]$Program,
        [string[]]$Arguments,
        [string]$Label
    )

    Write-Host "$Label..."
    Add-Content -LiteralPath $logPath -Encoding UTF8 -Value "=== $Label ==="
    # Windows PowerShell can surface native stderr as error records.
    $ErrorActionPreference = 'Continue'
    $PSNativeCommandUseErrorActionPreference = $false
    & $Program @Arguments 2>&1 |
        ForEach-Object {
            if ($_ -is [System.Management.Automation.ErrorRecord]) {
                $_.Exception.Message
            } else {
                $_
            }
        } |
        Out-File -LiteralPath $logPath -Append -Encoding UTF8 -ErrorAction Stop
    $code = $LASTEXITCODE

    if ($code -ne 0) {
        Get-Content -LiteralPath $logPath -Tail 35 | Write-Host
        Write-Host "FAIL $Label (exit $code)"
        exit $code
    }
}

Write-Host "Log: $logPath"
$previousTarget = $env:CARGO_TARGET_DIR
Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
try {
    $env:CARGO_TARGET_DIR = Join-Path $devRoot 'target'
    switch ($Mode) {
        'Test' {
            Invoke-Step $cargo @('test', '--locked', $Filter, '--', '--test-threads=1') "Test $Filter"
            $results = @(Select-String -LiteralPath $logPath -Pattern '^test result:' |
                ForEach-Object { $_.Line })
            $results | Write-Host
            if (-not ($results -match '^test result:.*\b[1-9]\d* passed\b')) {
                throw "No tests passed for filter '$Filter'. Check the filter and the log."
            }
        }
        'Launch' {
            Invoke-Step $cargo @('run', '--locked') 'Build and run the dev dock (quit the dock to finish)'
        }
        'Smoke' {
            Invoke-Step $cargo @('build', '--locked') 'Build the dev dock'
            Invoke-Step $cargo @('build', '--locked', '--example', 'desktop_smoke') 'Build the desktop diagnostic'
            $diagnostic = Join-Path $env:CARGO_TARGET_DIR 'debug\examples\desktop_smoke.exe'
            $dock = Join-Path $env:CARGO_TARGET_DIR 'debug\styled-dock.exe'
            $smokeArgs = @('--exe', $dock)
            if ($Fullscreen) { $smokeArgs += '--fullscreen' }
            if ($Taskbar) { $smokeArgs += '--taskbar' }
            Invoke-Step $diagnostic $smokeArgs 'Run desktop smoke'
            Select-String -LiteralPath $logPath -Pattern '^PASS all ' |
                ForEach-Object { Write-Host $_.Line }
        }
    }
    Write-Host "PASS $Mode"
}
finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    Pop-Location
}
