; Moonrise for Windows: a per-user installer (no administrator rights), unsigned for now.
; Built by build.ps1; run through ISCC with /DAppVersion=... /DPublishDir=... /DOutputDir=...

#define AppName "Moonrise"

[Setup]
AppId={{7F4F5C2A-3A0E-4E2B-9B6E-2F4C1A7D6B10}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Moonrise
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir={#OutputDir}
OutputBaseFilename=Moonrise-Setup-{#AppVersion}
SetupIconFile=..\Moonrise.App\Assets\Moonrise.ico
UninstallDisplayIcon={app}\Moonrise.exe
Compression=lzma2/normal
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
CloseApplications=yes

[Files]
Source: "{#PublishDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\Moonrise.exe"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\Moonrise.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Run]
Filename: "{app}\Moonrise.exe"; Description: "Open Moonrise"; Flags: nowait postinstall skipifsilent
