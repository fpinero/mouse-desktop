<#
.SYNOPSIS
    Install mouse-desktop for the current user, without Administrator privileges.

.DESCRIPTION
    Copies the executable into %LOCALAPPDATA%\Programs\mouse-desktop, optionally
    registers it to start at sign-in through HKEY_CURRENT_USER, and starts it.

    Nothing is written outside the user's own profile, no service or driver is installed,
    and no elevation is requested at any point.

.PARAMETER Source
    Path to mouse-desktop.exe. Defaults to the executable next to this script, then to
    target\release\mouse-desktop.exe in the repository.

.PARAMETER NoAutostart
    Install without registering the sign-in entry.

.PARAMETER NoStart
    Install without starting the utility afterwards.

.EXAMPLE
    .\install.ps1

.EXAMPLE
    .\install.ps1 -Source C:\Downloads\mouse-desktop.exe -NoAutostart
#>

[CmdletBinding()]
param(
    [string] $Source,
    [switch] $NoAutostart,
    [switch] $NoStart
)

$ErrorActionPreference = 'Stop'

$appName = 'mouse-desktop'
$targetDir = Join-Path $env:LOCALAPPDATA "Programs\$appName"
$targetExe = Join-Path $targetDir "$appName.exe"
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

function Resolve-Source {
    param([string] $Explicit)

    if ($Explicit) {
        if (-not (Test-Path -LiteralPath $Explicit)) {
            throw "No executable at $Explicit"
        }
        return (Resolve-Path -LiteralPath $Explicit).Path
    }

    $candidates = @(
        (Join-Path $PSScriptRoot "$appName.exe"),
        (Join-Path $PSScriptRoot "..\target\release\$appName.exe"),
        (Join-Path $PSScriptRoot "..\target\debug\$appName.exe")
    )

    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }

    throw "Could not find $appName.exe. Build it with 'cargo build --release' or pass -Source."
}

$sourceExe = Resolve-Source -Explicit $Source
Write-Host "Installing from $sourceExe"

# A running copy holds its own file open, so it has to go first.
Get-Process -Name $appName -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "Stopping the running copy (pid $($_.Id))"
    $_.CloseMainWindow() | Out-Null
    Start-Sleep -Milliseconds 300
    if (-not $_.HasExited) { Stop-Process -Id $_.Id -Force }
}

if (-not (Test-Path -LiteralPath $targetDir)) {
    New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
}
Copy-Item -LiteralPath $sourceExe -Destination $targetExe -Force
Write-Host "Installed to $targetExe"

if ($NoAutostart) {
    Write-Host "Skipping the sign-in entry (-NoAutostart)"
} else {
    # HKEY_CURRENT_USER is writable by a standard user, which is the whole point.
    New-ItemProperty -Path $runKey -Name $appName -Value "`"$targetExe`"" `
        -PropertyType String -Force | Out-Null
    Write-Host "Registered to start at sign-in"
}

if ($NoStart) {
    Write-Host "Not starting it (-NoStart)"
} else {
    Start-Process -FilePath $targetExe
    Write-Host "Started. Look for the icon in the notification area."
}

Write-Host ""
Write-Host "Hold the wheel button and drag left or right to change virtual desktop."
Write-Host "Configuration: $env:APPDATA\$appName\config.toml"
Write-Host "To remove it again, run uninstall.ps1"
