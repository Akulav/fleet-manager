; FleetView Agent installer (Inno Setup).
; Installs the agent exe, registers it as a LocalSystem auto-start service,
; and provisions agent.config.json (pinning the master identity).
; Built by CI; see .github/workflows/release.yml. Made by Akulav.

#define AppName "FleetView Agent"
#define AppPublisher "Akulav"
#define AppVersion GetEnv('FLEETVIEW_VERSION')
#if AppVersion == ""
  #define AppVersion "0.1.0"
#endif

[Setup]
AppId={{B4F3A9C1-2E7D-4B6A-9F10-7A1B2C3D4E5F}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\FleetView Agent
DisableProgramGroupPage=yes
DisableDirPage=yes
UninstallDisplayIcon={app}\fleetview-agent.exe
SetupIconFile=assets\agent.ico
OutputDir=..\dist
OutputBaseFilename=FleetViewAgent-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; The service runs as LocalSystem; installing it requires admin.
PrivilegesRequired=admin
ArchitecturesInstallIn64BitMode=x64compatible

[Files]
Source: "..\target\release\fleetview-agent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\agent.ico"; DestDir: "{app}"; Flags: ignoreversion
; Optional provisioning config placed next to this script by the master's
; "Agent installer config" export (or by CI). If present it is installed to
; %ProgramData%\FleetView so the service can read it on first start.
Source: "agent.config.json"; DestDir: "{commonappdata}\FleetView"; Flags: ignoreversion skipifsourcedoesntexist

[Dirs]
Name: "{commonappdata}\FleetView"; Permissions: admins-full system-full

[Run]
; Lock down the data directory: SYSTEM + Administrators only (the config holds
; the enrollment secret, and staged installers run as SYSTEM). Inno's
; Permissions only ADDS entries, so the inherited "Users: read / create files"
; from ProgramData must be removed explicitly. The service repeats this at start.
Filename: "{sys}\icacls.exe"; Parameters: """{commonappdata}\FleetView"" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F /T /C /Q"; Flags: runhidden waituntilterminated; StatusMsg: "Securing the FleetView data folder…"
; Register (or update) + start the service.
Filename: "{app}\fleetview-agent.exe"; Parameters: "install"; Flags: runhidden waituntilterminated; StatusMsg: "Installing the FleetView service…"

[UninstallRun]
Filename: "{app}\fleetview-agent.exe"; Parameters: "uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveService"
; The tray helpers run in user sessions from the same exe; close them so the
; file can be removed.
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM fleetview-agent.exe"; Flags: runhidden waituntilterminated; RunOnceId: "KillHelpers"

[UninstallDelete]
; Remove secrets and transient state. audit.log and logs\ are kept on purpose.
Type: files; Name: "{commonappdata}\FleetView\agent.config.json"
Type: files; Name: "{commonappdata}\FleetView\status.json"
Type: filesandordirs; Name: "{commonappdata}\FleetView\staging"

[Code]
// Stop the service and close tray helpers so the exe can be replaced.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\sc.exe'), 'stop FleetViewAgent', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  // Give the service a moment to stop (it also terminates its helper).
  Sleep(3000);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM fleetview-agent.exe', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  Result := '';
end;
