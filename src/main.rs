// En version finale (release), on cache la fenêtre de console noire sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use eframe::egui;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([420.0, 180.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Lecteur MP3",
        options,
        Box::new(|_cc| Ok(Box::new(LecteurApp::new()))),
    )
}

/// L'état de l'application.
struct LecteurApp {
    // La sortie son. Il faut la garder en vie tant que l'application tourne,
    // sinon le son s'arrête. On ne s'en sert pas directement, d'où le « _ ».
    _sortie: Option<OutputStream>,
    sortie_handle: Option<OutputStreamHandle>,
    // Le « lecteur » de rodio : on lui donne un morceau, il le joue.
    sink: Option<Sink>,
    // Le fichier actuellement chargé.
    fichier: Option<PathBuf>,
    // Un message affiché en bas de la fenêtre (erreurs, infos).
    message: String,
}

impl LecteurApp {
    fn new() -> Self {
        match OutputStream::try_default() {
            Ok((sortie, handle)) => Self {
                _sortie: Some(sortie),
                sortie_handle: Some(handle),
                sink: None,
                fichier: None,
                message: String::new(),
            },
            Err(e) => Self {
                _sortie: None,
                sortie_handle: None,
                sink: None,
                fichier: None,
                message: format!("Aucune sortie audio disponible : {e}"),
            },
        }
    }

    /// Ouvre la fenêtre Windows de choix de fichier, puis lance la lecture.
    fn ouvrir_fichier(&mut self) {
        let Some(chemin) = rfd::FileDialog::new()
            .add_filter("Fichiers MP3", &["mp3"])
            .pick_file()
        else {
            return; // l'utilisateur a annulé
        };

        match self.charger(&chemin) {
            Ok(()) => {
                self.fichier = Some(chemin);
                self.message.clear();
            }
            Err(e) => self.message = format!("Impossible de lire ce fichier : {e}"),
        }
    }

    /// Décode le fichier et le donne à un nouveau lecteur.
    fn charger(&mut self, chemin: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let handle = self
            .sortie_handle
            .as_ref()
            .ok_or("pas de sortie audio")?;

        let fichier = BufReader::new(File::open(chemin)?);
        let source = Decoder::new(fichier)?;

        // On arrête l'ancien morceau s'il y en avait un.
        if let Some(ancien) = self.sink.take() {
            ancien.stop();
        }

        let sink = Sink::try_new(handle)?;
        sink.append(source);
        self.sink = Some(sink);
        Ok(())
    }

    fn lecture(&mut self) {
        // Le morceau est encore chargé : on reprend là où on en était.
        if let Some(sink) = self.sink.as_ref().filter(|s| !s.empty()) {
            sink.play();
            return;
        }
        // Le morceau a été arrêté ou est terminé : on le recharge depuis le début.
        if let Some(chemin) = self.fichier.clone() {
            if let Err(e) = self.charger(&chemin) {
                self.message = format!("Impossible de lire ce fichier : {e}");
            }
        }
    }

    fn pause(&self) {
        if let Some(sink) = &self.sink {
            sink.pause();
        }
    }

    fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
    }

    /// Texte décrivant l'état actuel (en lecture, en pause, arrêté).
    fn etat(&self) -> &'static str {
        match &self.sink {
            Some(sink) if sink.empty() => "Arrêté",
            Some(sink) if sink.is_paused() => "En pause",
            Some(_) => "En lecture",
            None => "Arrêté",
        }
    }
}

impl eframe::App for LecteurApp {
    // Appelée à chaque image : on (re)dessine toute l'interface.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if ui.button("📂 Ouvrir un MP3…").clicked() {
                self.ouvrir_fichier();
            }

            ui.add_space(8.0);

            let nom = self
                .fichier
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Aucun fichier".to_string());
            ui.label(format!("Morceau : {nom}"));
            ui.label(format!("État : {}", self.etat()));

            ui.add_space(8.0);

            // Les boutons ne sont actifs que si un fichier a été choisi.
            ui.add_enabled_ui(self.fichier.is_some(), |ui| {
                ui.horizontal(|ui| {
                    if ui.button("▶ Lecture").clicked() {
                        self.lecture();
                    }
                    if ui.button("⏸ Pause").clicked() {
                        self.pause();
                    }
                    if ui.button("⏹ Stop").clicked() {
                        self.stop();
                    }
                });
            });

            if !self.message.is_empty() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::LIGHT_RED, &self.message);
            }
        });

        // Redessine régulièrement pour que l'état se mette à jour
        // quand le morceau se termine tout seul.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}
