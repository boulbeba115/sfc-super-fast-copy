; Super Fast Copy installer (Inno Setup 6). Build with build.ps1, or:
;   ISCC.exe installer\superfastcopy.iss

#define AppName "Super Fast Copy"
#define AppVersion "1.0.0"
#define AppExe "superfastcopy-gui.exe"
#define Gui "{app}\superfastcopy-gui.exe"
#define RepoUrl "https://github.com/boulbeba115/sfc-super-fast-copy"

[Setup]
AppId={{B1E7C4A2-5D3F-4F8E-9A6B-0C2D7E8F1A39}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=boulbeba115
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
AppUpdatesURL={#RepoUrl}/releases
DefaultDirName={autopf}\SuperFastCopy
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesEnvironment=yes
ChangesAssociations=yes
OutputDir=..\dist
OutputBaseFilename=SuperFastCopy-Setup-{#AppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\superfastcopy.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
LicenseFile=..\LICENSE
; Lets the installer replace the exes while a Super Fast Copy window is open.
CloseApplications=yes

[Tasks]
Name: "contextmenu"; Description: "Add ""Copy with SFC"" and ""Paste with SFC"" to the Explorer right-click menu"
Name: "sendto"; Description: "Add Super Fast Copy to the ""Send to"" menu"
Name: "addtopath"; Description: "Add the ""superfastcopy"" command to PATH"
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "classicmenu"; Description: "Windows 11: show the full right-click menu, so SFC appears without ""Show more options"" (applies to every app; restarts Explorer)"; Flags: unchecked
Name: "defender"; Description: "Exclude Super Fast Copy from Microsoft Defender real-time scanning (up to ~9x faster with many small files; files it writes are not scanned as they are copied)"; Flags: unchecked

[Files]
Source: "..\target\release\superfastcopy.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\superfastcopy-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon
Name: "{usersendto}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: sendto

[Registry]
; Remove menu entries left by pre-release builds (named FastCopy).
Root: HKA; Subkey: "Software\Classes\*\shell\FastCopy"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\FastCopy"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\FastPaste"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\FastPaste"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Drive\shell\FastPaste"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\*\shell\FastCopy.1Copy"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\FastCopy.1Copy"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\FastCopy.2Paste"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\FastCopy.2Paste"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Drive\shell\FastCopy.2Paste"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\*\shell\FastCopy.3CopyTo"; ValueType: none; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\FastCopy.3CopyTo"; ValueType: none; Flags: deletekey

; Right-click files/folders -> "Copy with SFC". Puts them on the clipboard
; (like Ctrl+C); Explorer runs one process per selected item and SFC merges
; them. MultiSelectModel=Player keeps the entry visible for selections of
; more than 15 items.
Root: HKA; Subkey: "Software\Classes\*\shell\SuperFastCopy.1Copy"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Copy with SFC"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\SuperFastCopy.1Copy"; ValueType: string; ValueName: "Icon"; ValueData: "{#Gui},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\SuperFastCopy.1Copy"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\SuperFastCopy.1Copy\command"; ValueType: string; ValueName: ""; ValueData: """{#Gui}"" --copy ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.1Copy"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Copy with SFC"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.1Copy"; ValueType: string; ValueName: "Icon"; ValueData: "{#Gui},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.1Copy"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.1Copy\command"; ValueType: string; ValueName: ""; ValueData: """{#Gui}"" --copy ""%1"""; Tasks: contextmenu

; "Paste with SFC": copies what's on the clipboard into the folder, with a
; progress window. Works for files copied with Ctrl+C too.
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\SuperFastCopy.2Paste"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Paste with SFC"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\SuperFastCopy.2Paste"; ValueType: string; ValueName: "Icon"; ValueData: "{#Gui},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\SuperFastCopy.2Paste\command"; ValueType: string; ValueName: ""; ValueData: """{#Gui}"" --paste ""%V"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.2Paste"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Paste into this folder with SFC"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.2Paste"; ValueType: string; ValueName: "Icon"; ValueData: "{#Gui},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.2Paste\command"; ValueType: string; ValueName: ""; ValueData: """{#Gui}"" --paste ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Drive\shell\SuperFastCopy.2Paste"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Paste onto this drive with SFC"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Drive\shell\SuperFastCopy.2Paste"; ValueType: string; ValueName: "Icon"; ValueData: "{#Gui},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Drive\shell\SuperFastCopy.2Paste\command"; ValueType: string; ValueName: ""; ValueData: """{#Gui}"" --paste ""%1"""; Tasks: contextmenu

; "SFC: copy to folder...": asks for a destination, then copies.
Root: HKA; Subkey: "Software\Classes\*\shell\SuperFastCopy.3CopyTo"; ValueType: string; ValueName: "MUIVerb"; ValueData: "SFC: copy to folder..."; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\SuperFastCopy.3CopyTo"; ValueType: string; ValueName: "Icon"; ValueData: "{#Gui},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\SuperFastCopy.3CopyTo\command"; ValueType: string; ValueName: ""; ValueData: """{#Gui}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.3CopyTo"; ValueType: string; ValueName: "MUIVerb"; ValueData: "SFC: copy to folder..."; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.3CopyTo"; ValueType: string; ValueName: "Icon"; ValueData: "{#Gui},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\SuperFastCopy.3CopyTo\command"; ValueType: string; ValueName: ""; ValueData: """{#Gui}"" ""%1"""; Tasks: contextmenu

; Windows 11 full right-click menu for the installing user (removed on uninstall).
Root: HKCU; Subkey: "Software\Classes\CLSID\{{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}"; ValueType: none; Flags: uninsdeletekeyifempty; Tasks: classicmenu
Root: HKCU; Subkey: "Software\Classes\CLSID\{{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32"; ValueType: string; ValueName: ""; ValueData: ""; Flags: uninsdeletekey; Tasks: classicmenu

; System PATH entry (removed again on uninstall, see [Code]).
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath(ExpandConstant('{app}')); Tasks: addtopath

[Run]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Add-MpPreference -ExclusionProcess '{app}\superfastcopy.exe','{app}\superfastcopy-gui.exe'"""; Flags: runhidden; StatusMsg: "Adding Microsoft Defender exclusion..."; Tasks: defender
Filename: "{cmd}"; Parameters: "/c taskkill /f /im explorer.exe & ping -n 3 127.0.0.1 >nul & (tasklist /fi ""imagename eq explorer.exe"" | find /i ""explorer.exe"" >nul || start explorer.exe)"; Flags: runhidden runasoriginaluser; StatusMsg: "Restarting Explorer..."; Tasks: classicmenu
Filename: "{app}\{#AppExe}"; Description: "Launch Super Fast Copy"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Harmless if the exclusion was never added.
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Remove-MpPreference -ExclusionProcess '{app}\superfastcopy.exe','{app}\superfastcopy-gui.exe'"""; Flags: runhidden; RunOnceId: "RemoveDefenderExclusion"

[Code]
const
  EnvKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';

function NeedsAddPath(Dir: string): Boolean;
var
  Path: string;
begin
  if not RegQueryStringValue(HKLM, EnvKey, 'Path', Path) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') = 0;
end;

procedure RemovePath(Dir: string);
var
  Path: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKLM, EnvKey, 'Path', Path) then
    exit;
  P := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';');
  if P = 0 then
    exit;
  { P is 1-based in the ';'-wrapped string, i.e. the separator before Dir. }
  if P = 1 then
    Delete(Path, 1, Length(Dir) + 1)
  else
    Delete(Path, P - 1, Length(Dir) + 1);
  RegWriteExpandStringValue(HKLM, EnvKey, 'Path', Path);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemovePath(ExpandConstant('{app}'));
end;
