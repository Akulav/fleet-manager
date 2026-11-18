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
; Lock down the data directory: SYSTEM + Administrators only (the config holds
; the enrollment secret). Users keep read/execute removed.
Name: "{commonappdata}\FleetView"; Permissions: admins-full system-full

[Run]
; Register + start the service.
Filename: "{app}\fleetview-agent.exe"; Parameters: "install"; Flags: runhidden waituntilterminated; StatusMsg: "Installing the FleetView service…"

[UninstallRun]
Filename: "{app}\fleetview-agent.exe"; Parameters: "uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveService"

[Code]
// Make sure any previous service instance is gone before reinstalling.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\sc.exe'), 'stop FleetViewAgent', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  Result := '';
end;
