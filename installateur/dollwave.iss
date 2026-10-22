; Installateur Windows du lecteur, fabriqué par Inno Setup (gratuit).
; GitHub le compile après le .exe (voir .github/workflows/release.yml) :
;   ISCC.exe /DVersion=0.2.0 installateur\dollwave.iss
; Il installe le lecteur pour l'utilisateur seul (pas besoin d'être
; administrateur), l'ajoute au menu Démarrer, propose un raccourci sur le
; bureau, l'inscrit parmi les lecteurs de .mp3, et se désinstalle depuis les
; Paramètres de Windows. Les réglages (%APPDATA%\dollwave) sont gardés.
; L'ancien « Lecteur MP3 » (avant le nouveau nom) est désinstallé tout seul
; s'il est encore là.

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
; Identifiant fixe : les nouvelles versions remplacent l'ancienne.
AppId={{AAB4BF3B-80BC-4D15-AB01-D2668695A96C}
AppName=Doll//Wave
AppVersion={#Version}
AppVerName=Doll//Wave {#Version}
AppPublisher=Luce
DefaultDirName={autopf}\DollWave
DefaultGroupName=DollWave
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Assistant sombre (Inno Setup 6.6 ou plus récent).
WizardStyle=modern dark
SetupIconFile=..\assets\icone\icone.ico
UninstallDisplayIcon={app}\dollwave.exe
UninstallDisplayName=Doll//Wave
OutputDir=..\target\installateur
OutputBaseFilename=dollwave-installation
Compression=lzma2
SolidCompression=yes
ChangesAssociations=yes

[Languages]
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "bureau"; Description: "Créer un raccourci sur le bureau"
Name: "mp3"; Description: "Proposer le lecteur pour ouvrir les fichiers .mp3"

[Files]
Source: "..\target\release\dollwave.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\DollWave"; Filename: "{app}\dollwave.exe"
Name: "{autodesktop}\DollWave"; Filename: "{app}\dollwave.exe"; Tasks: bureau

[Registry]
; Le type « Fichier MP3 » du lecteur : son icône et la commande d'ouverture.
; Windows ne laisse pas un programme se mettre tout seul par défaut : le
; lecteur apparaît dans « Ouvrir avec », et on le choisit une fois avec
; « Toujours ».
Root: HKA; Subkey: "Software\Classes\DollWave.mp3"; ValueType: string; ValueData: "Fichier MP3"; Flags: uninsdeletekey; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\DollWave.mp3\DefaultIcon"; ValueType: string; ValueData: """{app}\dollwave.exe"",0"; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\DollWave.mp3\shell\open\command"; ValueType: string; ValueData: """{app}\dollwave.exe"" ""%1"""; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\.mp3\OpenWithProgids"; ValueType: string; ValueName: "DollWave.mp3"; ValueData: ""; Flags: uninsdeletevalue; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\Applications\dollwave.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Doll//Wave"; Flags: uninsdeletekey; Tasks: mp3
Root: HKA; Subkey: "Software\Classes\Applications\dollwave.exe\SupportedTypes"; ValueType: string; ValueName: ".mp3"; ValueData: ""; Tasks: mp3

[Run]
Filename: "{app}\dollwave.exe"; Description: "Lancer Doll//Wave"; Flags: nowait postinstall skipifsilent

[Code]
// L'identifiant de l'ancien « Lecteur MP3 », avant le nouveau nom.
const
  AncienneCle = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{05D09422-12BE-43B6-AE72-5F4F2C91727B}_is1';

// Si l'ancien « Lecteur MP3 » est installé, on lance son désinstallateur
// sans fenêtre : il retire son .exe, ses raccourcis et son type « Fichier
// MP3 ». Les réglages ne sont pas touchés (Doll//Wave les reprend).
procedure DesinstallerLAncien(Racine: Integer);
var
  Commande: String;
  Resultat: Integer;
begin
  if RegQueryStringValue(Racine, AncienneCle, 'UninstallString', Commande) then
    Exec(RemoveQuotes(Commande), '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART', '',
      SW_HIDE, ewWaitUntilTerminated, Resultat);
end;

procedure CurStepChanged(Etape: TSetupStep);
begin
  if Etape = ssInstall then
  begin
    DesinstallerLAncien(HKCU);
    DesinstallerLAncien(HKLM);
  end;
end;
