# Offline checks: never downloads, replaces, or starts the dock.
$ErrorActionPreference = 'Stop'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'update.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
foreach ($name in @('StableVersion', 'SelectAsset', 'CheckBinary')) {
    $function = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }, $true)
    Invoke-Expression $function.Extent.Text
}
if ((StableVersion 'v1.10.0') -le (StableVersion '1.9.9')) { throw 'Version comparison must be numeric' }
foreach ($invalid in @('v1.0.0-beta', '1.0', '../1.0.0', '01.0.0', '1.0.0+build', '1.0.0;exit')) {
    $rejected = $false
    try { $null = StableVersion $invalid } catch { $rejected = $true }
    if (-not $rejected) { throw "Accepted invalid stable version: $invalid" }
}
Write-Host 'PASS updater PowerShell parsing and stable version selection'

$asset = [pscustomobject]@{ name = 'styled-dock.exe'; digest = 'sha256:' + ('a' * 64); browser_download_url = 'https://github.com/amacsizadam4/styled-dock/releases/download/v2.0.0/styled-dock.exe' }
$release = [pscustomobject]@{ assets = @($asset) }
$null = SelectAsset $release
foreach ($bad in @(
    [pscustomobject]@{ assets = @() },
    [pscustomobject]@{ assets = @($asset, $asset) },
    [pscustomobject]@{ assets = @([pscustomobject]@{ name = 'styled-dock.exe'; digest = ''; browser_download_url = $asset.browser_download_url }) },
    [pscustomobject]@{ assets = @([pscustomobject]@{ name = 'styled-dock.exe'; digest = $asset.digest; browser_download_url = 'https://example.com/evil.exe' }) }
)) {
    $rejected = $false
    try { $null = SelectAsset $bad } catch { $rejected = $true }
    if (-not $rejected) { throw 'Accepted invalid release asset' }
}
[byte[]]$binary = New-Object byte[] 128
$binary[0] = 77; $binary[1] = 90; $binary[60] = 64
$binary[64] = 80; $binary[65] = 69; $binary[68] = 100; $binary[69] = 134
CheckBinary $binary
$binary[69] = 1
$rejected = $false
try { CheckBinary $binary } catch { $rejected = $true }
if (-not $rejected) { throw 'Accepted wrong executable architecture' }
Write-Host 'PASS updater asset selection, digest requirement, URL restriction, and PE architecture checks'
