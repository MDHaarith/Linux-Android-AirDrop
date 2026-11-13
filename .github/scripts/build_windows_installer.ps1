# Builds target/release/airbridge-setup.exe from a release build of airbridge.exe.
# Run from linux/.
param([string]$Version = "0.0.0")
$ErrorActionPreference = "Stop"

# Microsoft's evergreen WebView2 bootstrapper; refuse anything not signed by Microsoft.
$bootstrapper = "target/release/MicrosoftEdgeWebview2Setup.exe"
Invoke-WebRequest "https://go.microsoft.com/fwlink/p/?LinkId=2124703" -OutFile $bootstrapper
$signature = Get-AuthenticodeSignature $bootstrapper
if ($signature.Status -ne "Valid" -or $signature.SignerCertificate.Subject -notmatch "O=Microsoft Corporation") {
    throw "WebView2 bootstrapper signature: $($signature.Status) $($signature.SignerCertificate.Subject)"
}

$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
    choco install innosetup --yes --no-progress
    if ($LASTEXITCODE -ne 0) { throw "installing Inno Setup failed" }
}
& $iscc /Qp "/DAppVersion=$Version" packaging/windows/airbridge.iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
Write-Output "Built target/release/airbridge-setup.exe ($Version)"
