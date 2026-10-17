; Inno Setup script for the Windows installer. The release workflow builds it with:
;   iscc /DAppVersion=1.2.3 /DExePath=C:\path\to\looji.exe installer\looji.iss
; It installs for the current user, with no administrator rights, so the program's folder is one the in-app updater
; can write to. Uninstalling leaves the user's data in %APPDATA%\looji alone.

#ifndef AppVersion
  #error AppVersion must be given on the command line: /DAppVersion=1.2.3
#endif
#ifndef ExePath
  #error ExePath must be given on the command line: /DExePath=C:\path\to\looji.exe
#endif

[Setup]
AppId={{6F2D7C4E-3B1A-4E58-9C07-2D5A8E41B7F3}
AppName=Looji
AppVersion={#AppVersion}
AppPublisher=Daniel Kosgei
AppPublisherURL=https://github.com/danielkosgei/looji
AppSupportURL=https://github.com/danielkosgei/looji/issues
DefaultDirName={autopf}\Looji
DefaultGroupName=Looji
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=looji-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
UninstallDisplayIcon={app}\looji.exe

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; DestName: "looji.exe"; Flags: ignoreversion

[Icons]
Name: "{group}\Looji"; Filename: "{app}\looji.exe"
Name: "{autodesktop}\Looji"; Filename: "{app}\looji.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\looji.exe"; Description: "Start Looji"; Flags: nowait postinstall skipifsilent
