# Copy latest adapters to the hstry config directory (overwrites existing).
# Windows equivalent of the justfile `update-adapters` recipe.
#
# Target directory (same resolution as dirs::config_dir / Config::default):
#   1. $env:XDG_CONFIG_HOME\hstry\adapters  (if set)
#   2. %APPDATA%\hstry\adapters             (Windows default)
#   3. ~/.config/hstry/adapters             (fallback)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$sourceAdapters = Join-Path $repoRoot "adapters"

if (-not (Test-Path -LiteralPath $sourceAdapters)) {
    throw "Source adapters directory not found: $sourceAdapters"
}

if ($env:XDG_CONFIG_HOME -and $env:XDG_CONFIG_HOME.Trim() -ne "") {
    $configRoot = $env:XDG_CONFIG_HOME.Trim()
}
elseif ($env:APPDATA -and $env:APPDATA.Trim() -ne "") {
    $configRoot = $env:APPDATA.Trim()
}
else {
    $configRoot = Join-Path $HOME ".config"
}

$adaptersDir = Join-Path $configRoot "hstry\adapters"

New-Item -ItemType Directory -Force -Path $adaptersDir | Out-Null

# Preserve local npm install (better-sqlite3); wipe only adapter sources.
Get-ChildItem -LiteralPath $adaptersDir -Force -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notin @("node_modules", "package.json", "package-lock.json") } |
    Remove-Item -Recurse -Force

Copy-Item -Path (Join-Path $sourceAdapters "*") -Destination $adaptersDir -Recurse -Force

Write-Host "Adapters updated in $adaptersDir"
Write-Host "Note: node_modules preserved. If better-sqlite3 is missing, run:"
Write-Host "  cd `"$adaptersDir`"; npm install better-sqlite3"
