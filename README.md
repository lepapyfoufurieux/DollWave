# lecteur_mp3

Un lecteur de fichiers MP3 pour Windows, écrit en Rust.

- Interface : [egui](https://github.com/emilk/egui) (via eframe)
- Audio : [rodio](https://github.com/RustAudio/rodio)

## Lancer l'application

Il faut avoir Rust installé (https://rustup.rs), puis dans ce dossier :

```
cargo run
```

Pour une version optimisée, sans fenêtre de console :

```
cargo run --release
```

## Créer le .exe

Pour obtenir un `lecteur_mp3.exe` autonome (qui se lance sans Rust) :

```
cargo build --release
```

Le fichier se trouve ensuite dans `target\release\lecteur_mp3.exe`. On peut le
copier où on veut et le partager : la police est intégrée dedans, et les
réglages sont rangés dans `%APPDATA%\lecteur_mp3`.

Le .exe est aussi compilé automatiquement par GitHub : pour chaque version
(étiquette `v0.1.0`, `v0.2.0`…), il est téléchargeable sur la page
« Releases » du dépôt.

## Avancement

- [x] Étape 1 : ouvrir et lire un MP3 (Lecture / Pause / Stop)
- [x] Étape 2 : réglage du volume
- [x] Étape 3 : barre de progression
- [x] Étape 4 : liste de lecture (précédent / suivant, lecture aléatoire)
- [x] Étape 5 : bibliothèque (tous les MP3 du dossier Musique, rangés par dossier, avec recherche ; un clic ajoute un morceau à la liste)
- [x] Étape 6 : favoris (★ sur un dossier ou un morceau pour le faire remonter en tête de son dossier)
