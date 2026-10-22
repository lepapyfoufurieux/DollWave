; Installateur Windows du lecteur, fabriqué par Inno Setup (gratuit).
; GitHub le compile après le .exe (voir .github/workflows/release.yml) :
;   ISCC.exe /DVersion=0.2.0 installateur\lecteur_mp3.iss
; Il installe le lecteur pour l'utilisateur seul (pas besoin d'être
; administrateur), l'ajoute au menu Démarrer, propose un raccourci sur le
; bureau, l'inscrit parmi les lecteurs de .mp3, et se désinstalle depuis les
; Paramètres de Windows. Les réglages (%APPDATA%\lecteur_mp3) sont gardés.

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
; Identifiant fixe : les nouvelles versions remplacent l'ancienne.
AppId={{05D09422-12BE-43B6-AE72-5F4F2C91727B}
AppName=Lecteur MP3
AppVersion={#Version}
AppVerName=Lecteur MP3 {#Version}
AppPublisher=Luce
DefaultDirName={autopf}\Lecteur MP3
DefaultGroupName=Lecteur MP3
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Assistant sombre (Inno Setup 6.6 ou plus récent).
WizardStyle=modern dark
SetupIconFile=..\assets\icone\icone.ico
UninstallDisplayIcon={app}\lecteur_mp3.exe
UninstallDisplayName=Lecteur MP3
OutputDir=..\target\installateur
OutputBaseFilename=lecteur_mp3-installation
Compression=lzma2
SolidCompression=yes
ChangesAssociations=yes

[Languages]
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "bureau"; Description: "Créer un raccourci sur le bureau"
Name: "mp3"; Description: "Proposer le lecteur pour ouvrir les fichiers .mp3"

[Files]
Source: "..\target\release\lecteur_mp3.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Lecteur MP3"; Filename: "{app}\lecteur_mp3.exe"
Name: "{autodesktop}\Lecteur MP3"; Filename: "{app}\lecteur_mp3.exe"; Tasks: bureau

[Registry]
; Le type « Fichier MP3 » du lecteur : son icône et la commande d'ouverture.
; Windows ne laisse pas un programme se mettre tout seul par défaut : le
; lecteur apparaît dans « Ouvrir avec », et on le choisit une fois avec
; « Toujours ».
Root: HKA; Subkey: "Software\Classes\LecteurMP3.mp3"; ValueType: string; ValueData: "Fichier MP3"; Flags: uninsdeletekey; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\LecteurMP3.mp3\DefaultIcon"; ValueType: string; ValueData: """{app}\lecteur_mp3.exe"",0"; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\LecteurMP3.mp3\shell\open\command"; ValueType: string; ValueData: """{app}\lecteur_mp3.exe"" ""%1"""; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\.mp3\OpenWithProgids"; ValueType: string; ValueName: "LecteurMP3.mp3"; ValueData: ""; Flags: uninsdeletevalue; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\Applications\lecteur_mp3.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Lecteur MP3"; Flags: uninsdeletekey; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\Applications\lecteur_mp3.exe\SupportedTypes"; ValueType: string; ValueName: ".mp3"; ValueData: ""; Tasks: mp3

[Run]
Filename: "{app}\lecteur_mp3.exe"; Description: "Lancer le Lecteur MP3"; Flags: nowait postinstall skipifsilent
