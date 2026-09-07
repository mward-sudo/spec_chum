; Inno Setup 6 script for Spec Chum (Refs #231).
;
; Compile via scripts/ci/build-windows-installer.ps1 (release CI), which passes:
;   MyAppVersion, StageDir, OutputDir, OutputBase
;
; Portable zip remains the unzip-and-run artifact; this builds the installer.

#ifndef MyAppVersion
  #error MyAppVersion must be defined (/DMyAppVersion=X.Y.Z)
#endif
#ifndef StageDir
  #error StageDir must be defined (/DStageDir=...)
#endif
#ifndef OutputDir
  #error OutputDir must be defined (/DOutputDir=...)
#endif
#ifndef OutputBase
  #error OutputBase must be defined (/DOutputBase=...)
#endif

#define MyAppName "Spec Chum"
#define MyAppExeName "spec_chum.exe"
#define MyAppPublisher "Spec Chum"
#define MyAppURL "https://github.com/mward-sudo/spec_chum"
; Stable AppId so upgrades replace the previous install (do not regenerate).
#define MyAppId "{{8E5C2A1B-4F3D-4A9E-9C7B-1D6E8F0A2B4C}"

[Setup]
AppId={#MyAppId}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
LicenseFile={#StageDir}\LICENSE
InfoBeforeFile={#StageDir}\README.txt
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBase}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExeName}
SetupIconFile=spec-chum.ico
DisableProgramGroupPage=yes
; Single primary binary; no ROMs in the installer (same as the portable zip).
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#StageDir}\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
