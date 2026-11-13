# Fails if airbridge.exe would not start on a clean Windows install (it needs
# the Visual C++ runtime DLLs) or has no embedded icon for shortcuts.
param([string]$Exe = "target/release/airbridge.exe")
$ErrorActionPreference = "Stop"
$path = (Resolve-Path $Exe).Path

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$dumpbin = & $vswhere -latest -products * -find "VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe" | Select-Object -First 1
if (-not $dumpbin) { throw "dumpbin.exe not found" }
$imports = & $dumpbin /nologo /dependents $path
$runtime = $imports | Where-Object { $_ -match '(?i)\b(vcruntime|msvcp)\d+' }
if ($runtime) { throw "airbridge.exe needs the Visual C++ runtime: $($runtime.Trim() -join ', ')" }

Add-Type -Namespace Native -Name Shell -MemberDefinition @'
[DllImport("shell32.dll", CharSet = CharSet.Unicode)]
public static extern uint ExtractIconEx(string file, int index, IntPtr[] large, IntPtr[] small, uint count);
'@
$icons = [Native.Shell]::ExtractIconEx($path, -1, $null, $null, 0)
if ($icons -lt 1) { throw "airbridge.exe has no embedded icon" }
Write-Output "airbridge.exe: $icons icon group(s), no Visual C++ runtime DLLs"
