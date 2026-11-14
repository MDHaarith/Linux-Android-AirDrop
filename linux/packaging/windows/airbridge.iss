; AirBridge installer (Inno Setup 6). Per-user: no admin prompt, and the
; in-app updater can replace airbridge.exe in its folder.
; Build: .github/scripts/build_windows_installer.ps1

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6B2E4F0A-9C1D-4E8B-A7F3-2D5C8B1E9A40}
AppName=AirBridge
AppVersion={#AppVersion}
AppPublisher=AirBridge
AppPublisherURL=https://github.com/MDHaarith/Airbridge
AppSupportURL=https://github.com/MDHaarith/Airbridge/issues
AppUpdatesURL=https://github.com/MDHaarith/Airbridge/releases
VersionInfoVersion={#AppVersion}
VersionInfoCompany=AirBridge
VersionInfoDescription=AirBridge installer
VersionInfoProductName=AirBridge
VersionInfoCopyright=MIT License
DefaultDirName={autopf}\AirBridge
DefaultGroupName=AirBridge
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SourceDir=..\..
OutputDir=target\release
OutputBaseFilename=airbridge-setup
SetupIconFile=packaging\windows\airbridge.ico
UninstallDisplayIcon={app}\airbridge.exe
; Close a running AirBridge before replacing it.
CloseApplications=yes
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: startup; Description: "Start AirBridge when I sign in (receives in the background)"
Name: sendto; Description: "Add AirBridge to the Explorer ""Send to"" menu"

[Files]
Source: "target\release\airbridge.exe"; DestDir: "{app}"; Flags: ignoreversion
; Microsoft's WebView2 bootstrapper, for the app window on systems without the runtime.
Source: "target\release\MicrosoftEdgeWebview2Setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Check: NeedsWebView2

[Icons]
Name: "{group}\AirBridge"; Filename: "{app}\airbridge.exe"
Name: "{sendto}\AirBridge"; Filename: "{app}\airbridge.exe"; Parameters: "send"; Tasks: sendto

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "AirBridge"; ValueData: """{app}\airbridge.exe"" daemon"; Tasks: startup; Flags: uninsdeletevalue

[Run]
Filename: "{tmp}\MicrosoftEdgeWebview2Setup.exe"; Parameters: "/silent /install"; StatusMsg: "Installing Microsoft Edge WebView2..."; Check: NeedsWebView2
Filename: "{app}\airbridge.exe"; Description: "Open AirBridge"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\airbridge.exe"; Parameters: "quit"; RunOnceId: "QuitAirBridge"; Flags: runhidden

[Code]
const
  WebView2Key = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

function HasVersion(Root: Integer; Key: String): Boolean;
var
  Version: String;
begin
  Result := RegQueryStringValue(Root, Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0');
end;

function NeedsWebView2: Boolean;
begin
  Result := not (HasVersion(HKLM, 'SOFTWARE\WOW6432Node\' + WebView2Key)
    or HasVersion(HKLM, 'SOFTWARE\' + WebView2Key)
    or HasVersion(HKCU, 'Software\' + WebView2Key));
end;
