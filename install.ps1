# Installs gdash for the current user, like any other app:
#   - builds the release exe (skip with -NoBuild)
#   - copies it to %LOCALAPPDATA%\Programs\gdash
#   - adds a Start Menu shortcut, so typing "gdash" in Start search finds it
#   - if "Start with Windows" is on, points it at the installed copy
# Run it again after changes to update. Remove with: .\install.ps1 -Uninstall
param([switch]$NoBuild, [switch]$Uninstall)
$ErrorActionPreference = 'Stop'

$dest = Join-Path $env:LOCALAPPDATA 'Programs\gdash'
$exe = Join-Path $dest 'gdash.exe'
$lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'gdash.lnk'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

if (Get-Process gdash -ErrorAction SilentlyContinue) {
    throw 'gdash is running. Exit it first (Esc > Exit gdash), then run this again.'
}

if ($Uninstall) {
    Remove-Item $lnk -ErrorAction SilentlyContinue
    Remove-ItemProperty $runKey -Name gdash -ErrorAction SilentlyContinue
    Remove-Item $dest -Recurse -Force -ErrorAction SilentlyContinue
    "Removed gdash. Your config and artwork cache are kept in %APPDATA%\gdash and %LOCALAPPDATA%\gdash."
    return
}

Push-Location $PSScriptRoot
try {
    if (-not $NoBuild) {
        $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item target\release\gdash.exe $dest -Force
    # ONNX Runtime ships DirectML.dll alongside; keep it next to the exe.
    if (Test-Path target\release\DirectML.dll) { Copy-Item target\release\DirectML.dll $dest -Force }
} finally {
    Pop-Location
}

$shell = New-Object -ComObject WScript.Shell
$s = $shell.CreateShortcut($lnk)
$s.TargetPath = $exe
$s.WorkingDirectory = $dest
$s.IconLocation = "$exe,0"
$s.Description = 'Game launcher'
$s.Save()

if (Get-ItemProperty $runKey -Name gdash -ErrorAction SilentlyContinue) {
    Set-ItemProperty $runKey -Name gdash -Value "`"$exe`""
}

"Installed to $dest"
"Start Menu shortcut: $lnk"
