; Paneltarium installer (Inno Setup 6).
;
; Built by .github/workflows/release.yml. By hand (after `cargo build --release`):
;   iscc /DAppVersion=0.1.0 installer\paneltarium.iss
; The installer lands in target\installer\.
;
; Per-user install: no admin prompt, installs to
; %LOCALAPPDATA%\Programs\Paneltarium, and "Start when I sign in" writes the
; same HKCU Run entry the app's own toggle uses, so either can turn it off.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppName "Paneltarium"
#define AppExe "paneltarium-app.exe"
#define Bin "..\target\release"

[Setup]
; Never change AppId: Windows uses it to find the existing install on upgrade.
AppId={{6F1C2B7A-5E4D-4C1B-9A3E-2D8F7B6C5A41}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Paneltarium
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=Paneltarium-{#AppVersion}-setup
SetupIconFile=..\assets\paneltarium.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Close a running Paneltarium so its files can be replaced on upgrade.
CloseApplications=force
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startup"; Description: "Start Paneltarium when I sign in"; GroupDescription: "Options:"
Name: "desktopicon"; Description: "Create a desktop shortcut for the profile manager"; GroupDescription: "Options:"; Flags: unchecked

[Files]
Source: "{#Bin}\paneltarium-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\paneltarium-manager.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\paneltarium.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{group}\Paneltarium"; Filename: "{app}\{#AppExe}"; Comment: "Start the Paneltarium tray app"
Name: "{group}\Paneltarium profile manager"; Filename: "{app}\paneltarium-manager.exe"
Name: "{autodesktop}\Paneltarium profile manager"; Filename: "{app}\paneltarium-manager.exe"; Tasks: desktopicon

[Registry]
; Same value the app's "Start with Windows" toggle reads and writes.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Paneltarium"; ValueData: """{app}\{#AppExe}"" --autostart"; Tasks: startup
; Remove it on uninstall even if it was switched on later from the app.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "Paneltarium"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueName: "Paneltarium"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\{#AppExe}"; Description: "Start Paneltarium now"; Flags: nowait postinstall skipifsilent
Filename: "{app}\paneltarium-manager.exe"; Description: "Open the profile manager"; Flags: nowait postinstall skipifsilent unchecked

[UninstallRun]
; Stop the tray app (and manager) so their files can be removed.
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM paneltarium-app.exe"; Flags: runhidden; RunOnceId: "StopApp"
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM paneltarium-manager.exe"; Flags: runhidden; RunOnceId: "StopManager"

; Profiles and settings in %APPDATA%\Paneltarium are kept on uninstall, so
; reinstalling (or upgrading) keeps your setups.
