; Inno Setup script for Papers (GNOME Document Viewer) on Windows.
; Compile with: iscc /DAppVersion=50.2 /DSourceDir=<bundle> /DOutputDir=<dir> papers.iss

#ifndef AppVersion
  #define AppVersion "0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\_work\dist\Papers"
#endif
#ifndef OutputDir
  #define OutputDir "..\_work\dist"
#endif

#define AppName "Papers"
#define AppExe "papers.exe"
#define ProgId "Papers.Document"

[Setup]
AppId={{8C3C7A51-7E0B-4C6B-9C59-3B0E8E2B6A51}
AppName={#AppName}
AppVerName={#AppName} {#AppVersion}
AppVersion={#AppVersion}
AppPublisher=The GNOME Project
AppPublisherURL=https://apps.gnome.org/Papers/
AppSupportURL=https://gitlab.gnome.org/GNOME/papers/-/issues
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
LicenseFile={#SourceDir}\share\licenses\papers\COPYING.txt
OutputDir={#OutputDir}
OutputBaseFilename=Papers-{#AppVersion}-windows-x64-setup
SetupIconFile={#SourceDir}\..\..\icon\papers.ico
UninstallDisplayIcon={app}\bin\{#AppExe}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequiredOverridesAllowed=dialog
ChangesAssociations=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Document Viewer"; Filename: "{app}\bin\{#AppExe}"; WorkingDir: "{app}\bin"; Comment: "View multi-page documents"
Name: "{autodesktop}\Document Viewer"; Filename: "{app}\bin\{#AppExe}"; WorkingDir: "{app}\bin"; Tasks: desktopicon

[Registry]
; ProgID used for "Open with" on the supported document types
Root: HKA; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueName: ""; ValueData: "Document"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\{#AppExe}"",0"
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\{#AppExe}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Document Viewer"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\{#AppExe}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".pdf"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.ai\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".ai"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.djvu\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".djvu"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.djv\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".djv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.tif\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".tif"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.tiff\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".tiff"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.cbz\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".cbz"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.cbr\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".cbr"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.cb7\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".cb7"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.cbt\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".cbt"; ValueData: ""

[Run]
Filename: "{app}\bin\{#AppExe}"; Description: "{cm:LaunchProgram,Document Viewer}"; Flags: nowait postinstall skipifsilent
