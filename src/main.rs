// En version finale (release), on cache la fenêtre de console noire sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Duration;

use eframe::egui;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([420.0, 220.0]),
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
    // La durée totale du morceau (inconnue pour certains fichiers).
    duree: Option<Duration>,
    // Pendant qu'on fait glisser la barre de progression, la position choisie
    // (en secondes). On ne se déplace dans le morceau qu'au relâchement.
    position_glissee: Option<f32>,
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
                duree: None,
                position_glissee: None,
                message: String::new(),
            },
            Err(e) => Self {
                _sortie: None,
                sortie_handle: None,
                sink: None,
                fichier: None,
                duree: None,
                position_glissee: None,
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
        // On lit la durée totale avant de donner le morceau au lecteur.
        let duree = source.total_duration();

        // On arrête l'ancien morceau s'il y en avait un.
        if let Some(ancien) = self.sink.take() {
            ancien.stop();
        }

        let sink = Sink::try_new(handle)?;
        sink.append(source);
        self.sink = Some(sink);
        self.duree = duree;
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

    /// Le temps écoulé depuis le début du morceau.
    fn position(&self) -> Duration {
        match &self.sink {
            Some(sink) if !sink.empty() => sink.get_pos(),
            _ => Duration::ZERO,
        }
    }

    /// Se déplace dans le morceau, à la position donnée (en secondes).
    fn aller_a(&mut self, secondes: f32) {
        if let Some(sink) = &self.sink {
            if let Err(e) = sink.try_seek(Duration::from_secs_f32(secondes)) {
                self.message = format!("Impossible de se déplacer dans le morceau : {e}");
            }
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

impl LecteurApp {
    /// La barre de progression : temps écoulé / durée totale, et un curseur
    /// qu'on peut faire glisser (ou cliquer) pour se déplacer dans le morceau.
    fn barre_de_progression(&mut self, ui: &mut egui::Ui) {
        let duree = self.duree.unwrap_or_default();
        // Pendant un glissement, on affiche la position choisie plutôt que la vraie.
        let mut secondes = self
            .position_glissee
            .unwrap_or_else(|| self.position().as_secs_f32())
            .min(duree.as_secs_f32());

        // Le curseur n'est utilisable que si un morceau est chargé et sa durée connue.
        let actif = self.duree.is_some() && self.sink.as_ref().is_some_and(|s| !s.empty());

        ui.add_enabled_ui(actif, |ui| {
            ui.horizontal(|ui| {
                // Le curseur prend toute la largeur, moins la place du texte des temps.
                ui.spacing_mut().slider_width = (ui.available_width() - 90.0).max(50.0);
                // Sans morceau, la plage serait vide (0 à 0) et egui placerait le
                // curseur au milieu : on garde au moins 1 seconde pour qu'il soit à gauche.
                let fin = duree.as_secs_f32().max(1.0);
                let reponse =
                    ui.add(egui::Slider::new(&mut secondes, 0.0..=fin).show_value(false));

                if reponse.dragged() {
                    // On glisse encore : on retient seulement la position.
                    self.position_glissee = Some(secondes);
                } else if self.position_glissee.take().is_some() || reponse.changed() {
                    // Relâchement après un glissement, ou simple clic : on s'y rend.
                    self.aller_a(secondes);
                }

                let total = match self.duree {
                    Some(d) => format_temps(d),
                    None => "--:--".to_string(),
                };
                ui.label(format!(
                    "{} / {total}",
                    format_temps(Duration::from_secs_f32(secondes))
                ));
            });
        });
    }
}

/// Écrit une durée sous la forme « minutes:secondes », par exemple 3:07.
fn format_temps(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
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
            self.barre_de_progression(ui);
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

        // Redessine régulièrement pour que le temps écoulé avance et que l'état
        // se mette à jour quand le morceau se termine tout seul.
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}
