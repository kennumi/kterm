; KTerm installer. Build: powershell installer\build.ps1  (needs Inno Setup 6.7+)
; Art comes from installer\make_art.py.

#define Exe "..\target\release\kterm.exe"
#define AppVersion GetStringFileInfo(Exe, "ProductVersion")
#define Small "art\small-100.png,art\small-125.png,art\small-150.png,art\small-175.png,art\small-200.png,art\small-225.png,art\small-250.png"
#define Welcome "art\welcome-100.png,art\welcome-125.png,art\welcome-150.png,art\welcome-175.png,art\welcome-200.png,art\welcome-225.png,art\welcome-250.png"

[Setup]
AppId={{95E0F76C-C3CD-4D38-9FFF-04EFD238D512}
AppName=KTerm
AppVersion={#AppVersion}
AppVerName=KTerm {#AppVersion}
AppPublisher=kennumi
VersionInfoVersion={#AppVersion}
VersionInfoDescription=KTerm Setup
; Per-user install into %LOCALAPPDATA%\Programs: no UAC prompt, same as KTerm's own font install.
PrivilegesRequired=lowest
DefaultDirName={autopf}\KTerm
DisableWelcomePage=no
DisableProgramGroupPage=yes
DisableReadyPage=yes
DisableDirPage=auto
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\dist
OutputBaseFilename=KTerm-Setup-{#AppVersion}
SetupIconFile=..\assets\kterm.ico
UninstallDisplayIcon={app}\kterm.exe
UninstallDisplayName=KTerm
Compression=lzma2/ultra64
SolidCompression=yes
; Look: built-in dark style on a flat background, no bevel lines; the mark carries the brand.
WizardStyle=modern dark hidebevels
WizardBackColor=#111317
WizardImageBackColor=#111317
WizardSmallImageBackColor=#111317
WizardImageFile={#Welcome}
WizardSmallImageFile={#Small}
; KTerm idles hidden in the background (Win+Enter), so upgrades must be allowed to stop it.
CloseApplications=force
RestartApplications=no

[Messages]
WelcomeLabel1=Welcome to KTerm
WelcomeLabel2=A fast, GPU-rendered terminal for Windows.%n%nPowerShell, cmd and Linux commands in one prompt, tiled windows, and Win+Enter from anywhere.%n%nKTerm installs just for you. No administrator rights needed.
FinishedHeadingLabel=KTerm is ready
FinishedLabelNoIcons=Press Win+Enter at any time to open a terminal.
FinishedLabel=Press Win+Enter at any time to open a terminal.
ClickFinish=
SelectTasksDesc=A couple of choices, then you're done.
SelectTasksLabel2=

[Tasks]
Name: autostart; Description: "Start with Windows, so Win+Enter always works"
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#Exe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\KTerm"; Filename: "{app}\kterm.exe"
Name: "{autodesktop}\KTerm"; Filename: "{app}\kterm.exe"; Tasks: desktopicon

[Registry]
; Same value `kterm --install` writes (src/win.rs).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "kterm"; ValueData: """{app}\kterm.exe"" --hidden"; Tasks: autostart; Flags: uninsdeletevalue
; Lets Win+R "kterm" and "ktm" find it without touching PATH.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\kterm.exe"; ValueType: string; ValueData: "{app}\kterm.exe"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\ktm.exe"; ValueType: string; ValueData: "{app}\kterm.exe"; Flags: uninsdeletekey

[Run]
Filename: "{app}\kterm.exe"; Description: "Open KTerm now"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Stop only the installed copy, not dev builds running from elsewhere.
Filename: "powershell.exe"; Parameters: "-NoProfile -Command ""Get-Process kterm -EA 0 | ? Path -eq '{app}\kterm.exe' | Stop-Process -Force"""; Flags: runhidden; RunOnceId: "StopKTerm"
