; FleetView Master installer (Inno Setup).
; Installs the desktop control app with Start Menu + optional desktop shortcut.
; Made by Akulav.

#define AppName "FleetView Master"
#define AppPublisher "Akulav"
#define AppVersion GetEnv('FLEETVIEW_VERSION')
#if AppVersion == ""
  #define AppVersion "0.1.0"
#endif

[Setup]
AppId={{C5A4B0D2-3F8E-4C7B-A021-8B2C3D4E5F60}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\FleetView Master
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\fleetview-master.exe
SetupIconFile=assets\master.ico
OutputDir=..\dist
OutputBaseFilename=FleetViewMaster-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional icons:"

[Files]
Source: "..\target\release\fleetview-master.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\master.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\FleetView Master"; Filename: "{app}\fleetview-master.exe"; IconFilename: "{app}\master.ico"
Name: "{autodesktop}\FleetView Master"; Filename: "{app}\fleetview-master.exe"; IconFilename: "{app}\master.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\fleetview-master.exe"; Description: "Launch FleetView Master"; Flags: nowait postinstall skipifsilent
