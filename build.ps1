# Builds Super Fast Copy and its installer: dist\SuperFastCopy-Setup-<version>.exe
#
# Needs: rustup (x86_64-pc-windows-gnu toolchain), WinLibs MinGW (for
# dlltool), Inno Setup 6. All three install with winget, see README.md.

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

# dlltool from WinLibs. Only its binutils folder goes on PATH, so Rust keeps
# using its own bundled linker and C runtime.
if (-not (Get-Command dlltool -ErrorAction SilentlyContinue)) {
    $dlltool = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BrechtSanders.WinLibs*\mingw64\x86_64-w64-mingw32\bin\dlltool.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $dlltool) { throw 'dlltool not found. Install it with: winget install BrechtSanders.WinLibs.POSIX.UCRT' }
    $env:Path = "$($dlltool.DirectoryName);$env:Path"
}

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup not found. Install it with: winget install JRSoftware.InnoSetup' }

& $iscc /Q installer\superfastcopy.iss
if ($LASTEXITCODE -ne 0) { throw 'installer build failed' }
Get-ChildItem dist\*.exe | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } }
