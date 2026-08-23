<#
.SYNOPSIS
    Remove mouse-desktop for the current user.

.DESCRIPTION
    Stops the utility, removes the sign-in entry and deletes the installation folder.
    Needs no Administrator privileges, because install.ps1 wrote nothing that would.

    The configuration file is kept unless -RemoveConfig is passed.

.PARAMETER RemoveConfig
    Also delete %APPDATA%\mouse-desktop, which holds the configuration and the log.

.EXAMPLE
    .\uninstall.ps1

.EXAMPLE
    .\uninstall.ps1 -RemoveConfig
#>

[CmdletBinding()]
param(
    [switch] $RemoveConfig
)

$ErrorActionPreference = 'Stop'

$appName = 'mouse-desktop'
$targetDir = Join-Path $env:LOCALAPPDATA "Programs\$appName"
$configDir = Join-Path $env:APPDATA $appName
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

Get-Process -Name $appName -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "Stopping the running copy (pid $($_.Id))"
    $_.CloseMainWindow() | Out-Null
    Start-Sleep -Milliseconds 300
    if (-not $_.HasExited) { Stop-Process -Id $_.Id -Force }
}

if (Get-ItemProperty -Path $runKey -Name $appName -ErrorAction SilentlyContinue) {
    Remove-ItemProperty -Path $runKey -Name $appName
    Write-Host "Removed the sign-in entry"
}

if (Test-Path -LiteralPath $targetDir) {
    Remove-Item -LiteralPath $targetDir -Recurse -Force
    Write-Host "Deleted $targetDir"
} else {
    Write-Host "Nothing installed at $targetDir"
}

if ($RemoveConfig) {
    if (Test-Path -LiteralPath $configDir) {
        Remove-Item -LiteralPath $configDir -Recurse -Force
        Write-Host "Deleted $configDir"
    }
} elseif (Test-Path -LiteralPath $configDir) {
    Write-Host "Kept the configuration in $configDir, pass -RemoveConfig to delete it too"
}

Write-Host "Done."
