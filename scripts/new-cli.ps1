#!/usr/bin/env pwsh

[CmdletBinding()]
param(
    [Parameter(Position = 0, Mandatory = $true)]
    [ValidatePattern('^[a-zA-Z][a-zA-Z0-9_-]*$')]
    [string]$Name,

    [Parameter()]
    [string]$Path
)

function Write-Usage {
    Write-Host "Usage: new-cli.ps1 <name> [-Path <destination>]" -ForegroundColor Cyan
    Write-Host "Creates a new workspace project by cloning this template." -ForegroundColor Cyan
    Write-Host "Renames all crates from rust-* to <name>-* pattern." -ForegroundColor Cyan
}

try {
    if ($PSBoundParameters.ContainsKey('Name') -eq $false) {
        Write-Usage
        throw 'Project name is required.'
    }

    $scriptDir = Split-Path -Parent $PSCommandPath
    $templateRoot = Split-Path -Parent $scriptDir

    if ([string]::IsNullOrWhiteSpace($Path)) {
        $parentDir = Split-Path -Parent $templateRoot
        $destination = Join-Path -Path $parentDir -ChildPath $Name
    } else {
        $destination = if ([System.IO.Path]::IsPathRooted($Path)) { $Path } else { Join-Path -Path (Get-Location) -ChildPath $Path }
    }

    if (Test-Path -LiteralPath $destination) {
        throw "Destination already exists: $destination"
    }

    New-Item -ItemType Directory -Path $destination | Out-Null

    $excluded = @('.git', 'target', '.DS_Store')

    function Copy-Template {
        param(
            [string]$Source,
            [string]$Dest
        )

        Get-ChildItem -LiteralPath $Source -Force | ForEach-Object {
            if ($excluded -contains $_.Name) {
                return
            }

            $targetPath = Join-Path -Path $Dest -ChildPath $_.Name

            if ($_.PSIsContainer) {
                if (-not (Test-Path -LiteralPath $targetPath)) {
                    New-Item -ItemType Directory -Path $targetPath | Out-Null
                }
                Copy-Template -Source $_.FullName -Dest $targetPath
            } else {
                Copy-Item -LiteralPath $_.FullName -Destination $targetPath -Force
            }
        }
    }

    Copy-Template -Source $templateRoot -Dest $destination

    # Replacement values
    $oldWorkspace = 'rust-workspace'
    $oldPrefix = 'rust-'
    $newPrefix = "$Name-"
    $oldEnvPrefix = 'RUST_WORKSPACE'
    $newEnvPrefix = $Name.ToUpper() -replace '-', '_'

    # Files to update
    $filesToUpdate = @(
        'Cargo.toml',
        'Cargo.lock',
        'README.md',
        'AGENTS.md',
        'TUI.md',
        'examples/config.toml'
    )

    # Add crate files
    $cratesDir = Join-Path -Path $destination -ChildPath 'crates'
    if (Test-Path -LiteralPath $cratesDir) {
        Get-ChildItem -LiteralPath $cratesDir -Directory | ForEach-Object {
            $filesToUpdate += "crates/$($_.Name)/Cargo.toml"
            $mainRs = "crates/$($_.Name)/src/main.rs"
            $libRs = "crates/$($_.Name)/src/lib.rs"
            if (Test-Path -LiteralPath (Join-Path -Path $destination -ChildPath $mainRs)) {
                $filesToUpdate += $mainRs
            }
            if (Test-Path -LiteralPath (Join-Path -Path $destination -ChildPath $libRs)) {
                $filesToUpdate += $libRs
            }
        }
    }

    # Update file contents
    foreach ($relative in $filesToUpdate) {
        $filePath = Join-Path -Path $destination -ChildPath $relative
        if (Test-Path -LiteralPath $filePath) {
            $content = Get-Content -LiteralPath $filePath -Raw
            $updated = $content `
                -replace [regex]::Escape($oldWorkspace), $Name `
                -replace [regex]::Escape($oldPrefix), $newPrefix `
                -replace [regex]::Escape($oldEnvPrefix), $newEnvPrefix
            Set-Content -LiteralPath $filePath -Value $updated -Encoding UTF8
        }
    }

    # Rename crate directories
    if (Test-Path -LiteralPath $cratesDir) {
        Get-ChildItem -LiteralPath $cratesDir -Directory | Where-Object { $_.Name.StartsWith($oldPrefix) } | ForEach-Object {
            $newName = $newPrefix + $_.Name.Substring($oldPrefix.Length)
            $newPath = Join-Path -Path $cratesDir -ChildPath $newName
            Rename-Item -LiteralPath $_.FullName -NewName $newName
        }
    }

    Write-Host "Created workspace project at $destination" -ForegroundColor Green
    Write-Host "Crates renamed to: $Name-core, $Name-cli, $Name-tui, $Name-mcp, $Name-api" -ForegroundColor Green
    exit 0
}
catch {
    Write-Error $_
    Write-Usage
    exit 1
}
