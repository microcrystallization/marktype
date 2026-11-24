; MarkType Windows Installer Script (Inno Setup 6/7)
; Build via: ISCC.exe packaging\windows\marktype.iss

#define MyAppName "MarkType"
#ifndef MyAppVersion
  #define MyAppVersion "0.1.2"
#endif
#ifndef BuildDir
  #define BuildDir "..\..\target\release"
#endif
#define MyAppPublisher "MarkType Contributors"
#define MyAppURL "https://github.com/microcrystallization/marktype"
#define MyAppExeName "marktype.exe"

[Setup]
AppId={{8F93B2A1-4E5D-4C2B-9A71-3F8C2E61B904}
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
LicenseFile=..\..\LICENSE
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\..\dist
OutputBaseFilename=MarkType-Setup-{#MyAppVersion}-x64
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\MarkType.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern dynamic
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
DisableProgramGroupPage=yes

[Languages]
Name: "chinesesimplified"; MessagesFile: "languages\ChineseSimplified.isl"
Name: "chinesetraditional"; MessagesFile: "languages\ChineseTraditional.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[CustomMessages]
chinesesimplified.AssocMd=将 .md / .markdown 文件关联到 MarkType
chinesesimplified.ContextMenu=在右键菜单添加“使用 MarkType 打开”
chinesetraditional.AssocMd=將 .md / .markdown 檔案關聯到 MarkType
chinesetraditional.ContextMenu=在右鍵選單新增「使用 MarkType 開啟」
english.AssocMd=Associate .md and .markdown files with MarkType
english.ContextMenu=Add "Open with MarkType" to Explorer context menu
japanese.AssocMd=.md / .markdown ファイルを MarkType に関連付ける
japanese.ContextMenu=右クリックメニューに「MarkType で開く」を追加
korean.AssocMd=.md / .markdown 파일을 MarkType에 연결
korean.ContextMenu=탐색기 컨텍스트 메뉴에 "MarkType으로 열기" 추가
french.AssocMd=Associer les fichiers .md et .markdown à MarkType
french.ContextMenu=Ajouter « Ouvrir avec MarkType » au menu contextuel

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "assocmd"; Description: "{cm:AssocMd}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "contextmenu"; Description: "{cm:ContextMenu}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#BuildDir}\marktype.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BuildDir}\WebView2Loader.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\icon.ico"; DestDir: "{app}"; DestName: "MarkType.ico"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\NOTICE.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "languages\LICENSE-InnoSetup.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\examples\getting-started.md"; DestDir: "{app}\examples"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\MarkType.ico"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\MarkType.ico"; Tasks: desktopicon

[Registry]
; Associate .md and .markdown files with MarkType
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "MarkType.Markdown"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assocmd
Root: HKA; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueType: string; ValueName: "MarkType.Markdown"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assocmd
Root: HKA; Subkey: "Software\Classes\MarkType.Markdown"; ValueType: string; ValueName: ""; ValueData: "Markdown Document"; Flags: uninsdeletekey; Tasks: assocmd
Root: HKA; Subkey: "Software\Classes\MarkType.Markdown\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\MarkType.ico"",0"; Tasks: assocmd
Root: HKA; Subkey: "Software\Classes\MarkType.Markdown\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: assocmd

; Explorer right-click "Open with MarkType" for any file
Root: HKA; Subkey: "Software\Classes\*\shell\OpenWithMarkType"; ValueType: string; ValueName: ""; ValueData: "Open with MarkType"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\OpenWithMarkType"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\MarkType.ico"",0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\OpenWithMarkType\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
