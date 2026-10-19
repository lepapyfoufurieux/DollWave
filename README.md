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

## Avancement

- [x] Étape 1 : ouvrir et lire un MP3 (Lecture / Pause / Stop)
- [ ] Étape 2 : réglage du volume
- [ ] Étape 3 : barre de progression
- [ ] Étape 4 : liste de lecture
