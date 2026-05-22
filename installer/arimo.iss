#define MyAppName      "Arimo"
#define MyAppVersion   "1.0.0"
#define MyAppPublisher "Egecan Akıncıoğlu"
#define MyAppURL       "https://github.com/egecanakincioglu/arimo"
#define MyAppExeName   "arc.exe"
#define MyAppDir       "D:\Desktop Items\Planlanan Projeler\arimo-bootstrap"
#define MyArimoDir     "D:\Desktop Items\Planlanan Projeler\arimo"

[Setup]
AppId={{A7F3C2D1-4E8B-4F9A-B2C3-D4E5F6A7B8C9}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\Arimo
DefaultGroupName=Arimo
AllowNoIcons=yes
LicenseFile={#MyArimoDir}\LICENSE
OutputDir={#MyAppDir}\installer\output
OutputBaseFilename=arimo-{#MyAppVersion}-windows-x64-setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "addtopath"; Description: "Add arc to system PATH (recommended)"; GroupDescription: "Additional tasks:"

[Files]
Source: "{#MyArimoDir}\arc.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyAppDir}\stdlib\*"; DestDir: "{app}\stdlib"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Arimo Documentation"; Filename: "{app}\arc.exe"; Parameters: "--help"
Name: "{group}\Uninstall Arimo"; Filename: "{uninstallexe}"

[Registry]
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: PathNotInPath('{app}'); Tasks: addtopath

[Code]
function PathNotInPath(Path: string): Boolean;
var
  CurrentPath: string;
begin
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE,
    'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
    'Path', CurrentPath) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(Lowercase(Path), Lowercase(CurrentPath)) = 0;
end;

[Run]
Filename: "cmd.exe"; Parameters: "/c setx PATH ""%PATH%;{app}"" /M"; StatusMsg: "Updating PATH..."; Flags: runhidden; Tasks: addtopath

[UninstallRun]
Filename: "cmd.exe"; Parameters: "/c for /f ""tokens=*"" %i in ('echo %PATH%') do setx PATH ""%i"" /M"; Flags: runhidden

[Messages]
WelcomeLabel2=This will install [name/ver] on your computer.%n%nArimo is a statically-typed, object-oriented systems programming language with its own IR, three-layer memory safety, and native code generation.%n%nIt is recommended that you close all other applications before continuing.
FinishedLabel=Setup has finished installing [name] on your computer.%n%nThe arc compiler is now available in your terminal.%n%nType: arc --version
