; Vestra Player installer (Inno Setup 6). Built by packaging/bundle-windows.mjs,
; which passes the defines below with /D. Per-user install without
; administrator rights (an all-users install can be chosen in the dialog);
; the user's data in %APPDATA%\Vestra stays on uninstall unless the user
; asks to delete it (coordinator decision 6). No file associations yet
; (decision 10).

#ifndef AppVersion
  #error Build with packaging/bundle-windows.mjs (AppVersion and the other defines are missing)
#endif

[Setup]
AppId={#AppId}
AppName=Vestra Player
AppVersion={#AppVersion}
AppVerName=Vestra Player {#AppVersion}
AppPublisher=Vestra Player contributors
AppPublisherURL=https://github.com/vestrasys/vestra-player
AppSupportURL=https://github.com/vestrasys/vestra-player
VersionInfoVersion={#AppVersionNumeric}
VersionInfoProductName=Vestra Player
VersionInfoDescription=Vestra Player Setup
DefaultDirName={autopf}\Vestra Player
DefaultGroupName=Vestra Player
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
SetupIconFile={#IconFile}
UninstallDisplayIcon={app}\vestra-host.exe
UninstallDisplayName=Vestra Player
LicenseFile={#SourceDir}\licenses\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
; TODO(S7 signing): SignTool=… (signs setup.exe and the uninstaller), in the
; release environment only; see release.yml sign-windows.

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; An upgrade replaces the shipped resources completely (no stale plugins or UI files).
Type: filesandordirs; Name: "{app}\resources"
Type: filesandordirs; Name: "{app}\licenses"
Type: filesandordirs; Name: "{app}\node"
Type: filesandordirs; Name: "{app}\ffmpeg"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Vestra Player"; Filename: "{app}\vestra-host.exe"
Name: "{autodesktop}\Vestra Player"; Filename: "{app}\vestra-host.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\vestra-host.exe"; Description: "{cm:LaunchProgram,Vestra Player}"; Flags: nowait postinstall skipifsilent

[Code]
// Plugins, plugin data and settings live in %APPDATA%\Vestra. Keep them
// unless the user says otherwise; silent uninstalls always keep them.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Data: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    Data := ExpandConstant('{userappdata}\Vestra');
    if DirExists(Data) and not UninstallSilent then
      if MsgBox('Also delete your Vestra Player plugins and settings?' + #13#10 + Data,
                mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(Data, True, True, True);
  end;
end;
