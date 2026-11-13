# Fails if airbridge.exe would not start on a clean Windows install (it needs
# the Visual C++ runtime DLLs).
param([string]$Exe = "target/release/airbridge.exe")
$ErrorActionPreference = "Stop"
$path = (Resolve-Path $Exe).Path

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$dumpbin = & $vswhere -latest -products * -find "VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe" | Select-Object -First 1
if (-not $dumpbin) { throw "dumpbin.exe not found" }
$imports = & $dumpbin /nologo /dependents $path
$runtime = $imports | Where-Object { $_ -match '(?i)\b(vcruntime|msvcp)\d+' }
if ($runtime) { throw "airbridge.exe needs the Visual C++ runtime: $($runtime.Trim() -join ', ')" }
Write-Output "airbridge.exe: no Visual C++ runtime DLLs"
