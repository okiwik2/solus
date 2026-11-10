; Solus installer (Inno Setup 6). Per-user install: no administrator rights needed.
; Build: ISCC.exe /DAppVersion=1.0.0 installer\solus.iss

#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif
#define AppName "Solus"
#define AppExe "solus.exe"
#define Root ".."

[Setup]
AppId={{6B1F2C1E-5A37-4C2B-9C1D-50D7A5B2E0A1}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Solus
AppPublisherURL=https://github.com/okiwik2/solus
VersionInfoVersion={#AppVersion}
VersionInfoDescription=Solus browser installer
DefaultDirName={localappdata}\Programs\Solus
DisableDirPage=auto
DisableProgramGroupPage=yes
DefaultGroupName=Solus
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.22000
OutputDir={#Root}\dist
OutputBaseFilename=SolusSetup-{#AppVersion}
SetupIconFile={#Root}\res\solus.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
WizardSizePercent=110
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=force
RestartApplications=no
ChangesAssociations=yes
LicenseFile={#Root}\installer\license.txt

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#Root}\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\ui\*"; DestDir: "{app}\ui"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Root}\res\lists\*.txt"; DestDir: "{app}\lists"; Flags: ignoreversion
Source: "{#Root}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\docs\USER-GUIDE.md"; DestDir: "{app}"; DestName: "User guide.md"; Flags: ignoreversion
Source: "{#Root}\installer\redist\MicrosoftEdgeWebview2Setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Check: NeedsWebView2

[Icons]
Name: "{autoprograms}\Solus"; Filename: "{app}\{#AppExe}"; AppUserModelID: "Solus.Browser"
Name: "{autodesktop}\Solus"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon; AppUserModelID: "Solus.Browser"

[Registry]
; Register Solus as a browser Windows can offer as the default (Settings > Apps > Default apps)
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus"; ValueType: string; ValueName: ""; ValueData: "Solus"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"""
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "Solus"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "A fast, private, low-memory web browser"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: "{app}\{#AppExe},0"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities\StartMenu"; ValueType: string; ValueName: "StartMenuInternet"; ValueData: "Solus"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities\URLAssociations"; ValueType: string; ValueName: "http"; ValueData: "SolusURL"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities\URLAssociations"; ValueType: string; ValueName: "https"; ValueData: "SolusURL"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".htm"; ValueData: "SolusHTML"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".html"; ValueData: "SolusHTML"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".pdf"; ValueData: "SolusHTML"
Root: HKCU; Subkey: "Software\Clients\StartMenuInternet\Solus\Capabilities\FileAssociations"; ValueType: string; ValueName: ".svg"; ValueData: "SolusHTML"
Root: HKCU; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "Solus"; ValueData: "Software\Clients\StartMenuInternet\Solus\Capabilities"; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\SolusURL"; ValueType: string; ValueName: ""; ValueData: "Solus URL"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\SolusURL"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\SolusURL"; ValueType: string; ValueName: "FriendlyTypeName"; ValueData: "Solus URL"
Root: HKCU; Subkey: "Software\Classes\SolusURL\Application"; ValueType: string; ValueName: "AppUserModelId"; ValueData: "Solus.Browser"
Root: HKCU; Subkey: "Software\Classes\SolusURL\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"
Root: HKCU; Subkey: "Software\Classes\SolusURL\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" -- ""%1"""
Root: HKCU; Subkey: "Software\Classes\SolusHTML"; ValueType: string; ValueName: ""; ValueData: "Solus HTML Document"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\SolusHTML\Application"; ValueType: string; ValueName: "AppUserModelId"; ValueData: "Solus.Browser"
Root: HKCU; Subkey: "Software\Classes\SolusHTML\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"
Root: HKCU; Subkey: "Software\Classes\SolusHTML\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" -- ""%1"""

[Run]
; WebView2 ships with Windows 11; this only runs if it was removed
Filename: "{tmp}\MicrosoftEdgeWebview2Setup.exe"; Parameters: "/silent /install"; StatusMsg: "Installing the Microsoft Edge WebView2 Runtime..."; Check: NeedsWebView2; Flags: waituntilterminated
; interactive installs leave the first launch to the user (unticked by default)
Filename: "{app}\{#AppExe}"; Description: "Open Solus now"; Flags: postinstall nowait skipifsilent unchecked
; silent auto-updates started with "Restart now" reopen Solus
Filename: "{app}\{#AppExe}"; Flags: nowait runasoriginaluser; Check: RelaunchRequested

[UninstallRun]
Filename: "{cmd}"; Parameters: "/C taskkill /F /IM {#AppExe}"; Flags: runhidden; RunOnceId: "StopSolus"

[Code]
function NeedsWebView2(): Boolean;
var
  V: String;
begin
  Result := not (RegQueryStringValue(HKLM, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}', 'pv', V) and (V <> '') and (V <> '0.0.0.0'))
        and not (RegQueryStringValue(HKCU, 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}', 'pv', V) and (V <> '') and (V <> '0.0.0.0'));
end;

function RelaunchRequested(): Boolean;
begin
  Result := WizardSilent() and (ExpandConstant('{param:RELAUNCH|0}') = '1');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    DataDir := ExpandConstant('{localappdata}\Solus');
    if DirExists(DataDir) and not UninstallSilent() then
      if MsgBox('Also delete your Solus data (bookmarks, history, passwords, settings and backups)?' + #13#10 + #13#10 +
                'Choose No to keep it, for example if you plan to reinstall Solus.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(DataDir, True, True, True);
  end;
end;
