// En version finale (release), on cache la fenêtre de console noire sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod platine;
mod visualiseur;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use rodio::{OutputStream, OutputStreamHandle, Sink};

use platine::{Morceau, Telecommande};
use visualiseur::Visualiseur;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([520.0, 480.0]),
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
    // Le « lecteur » de rodio : on lui donne la platine, il envoie son son
    // vers les haut-parleurs.
    sink: Option<Sink>,
    // La télécommande de la platine en cours (pause, position, scratch).
    telecommande: Option<Telecommande>,
    // Le fichier actuellement chargé, et son son décodé en mémoire.
    fichier: Option<PathBuf>,
    morceau: Option<Arc<Morceau>>,
    // Pendant qu'on fait glisser la barre de progression, la position de la
    // souris (en secondes). La musique la suit, comme un scratch de vinyle.
    position_glissee: Option<f32>,
    // Le volume choisi avec le curseur, de 0.0 (rien) à 1.0 (plein volume).
    volume: f32,
    // Le son est-il coupé (bouton muet) ? On garde le volume choisi à côté
    // pour le retrouver tel quel quand on remet le son.
    muet: bool,
    // L'image qui bouge avec la musique.
    visualiseur: Visualiseur,
    // Un message affiché en bas de la fenêtre (erreurs, infos).
    message: String,
}

impl LecteurApp {
    fn new() -> Self {
        let (sortie, sortie_handle, message) = match OutputStream::try_default() {
            Ok((sortie, handle)) => (Some(sortie), Some(handle), String::new()),
            Err(e) => (None, None, format!("Aucune sortie audio disponible : {e}")),
        };
        Self {
            _sortie: sortie,
            sortie_handle,
            sink: None,
            telecommande: None,
            fichier: None,
            morceau: None,
            position_glissee: None,
            volume: 0.8,
            muet: false,
            visualiseur: Visualiseur::new(),
            message,
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

    /// Décode tout le fichier en mémoire, puis lance la lecture depuis le début.
    fn charger(&mut self, chemin: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let morceau = Arc::new(Morceau::charger(chemin)?);
        self.morceau = Some(morceau);
        self.demarrer()
    }

    /// Pose le morceau déjà décodé sur une nouvelle platine et la lance.
    fn demarrer(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // On arrête l'ancien morceau s'il y en avait un.
        self.stop();

        let handle = self.sortie_handle.as_ref().ok_or("pas de sortie audio")?;
        let morceau = self.morceau.clone().ok_or("aucun morceau chargé")?;

        let (telecommande, platine) = Telecommande::nouvelle(morceau);
        let sink = Sink::try_new(handle)?;
        sink.append(platine);
        self.sink = Some(sink);
        self.appliquer_volume();
        self.telecommande = Some(telecommande);
        Ok(())
    }

    /// Transmet le volume choisi (ou le silence, si muet) au lecteur rodio.
    fn appliquer_volume(&self) {
        if let Some(sink) = &self.sink {
            // L'oreille n'entend pas le volume de façon « linéaire » : avec le
            // curseur à moitié, le son paraîtrait presque aussi fort qu'au maximum.
            // En mettant la valeur au carré, chaque cran du curseur s'entend pareil.
            let volume = if self.muet {
                0.0
            } else {
                self.volume * self.volume
            };
            sink.set_volume(volume);
        }
    }

    /// La platine en cours, si un morceau est en lecture ou en pause.
    fn platine_active(&self) -> Option<&Telecommande> {
        match &self.sink {
            Some(sink) if !sink.empty() => self.telecommande.as_ref(),
            _ => None,
        }
    }

    fn lecture(&mut self) {
        // Le morceau est encore en cours : on reprend là où on en était.
        if let Some(telecommande) = self.platine_active() {
            telecommande.mettre_en_pause(false);
            return;
        }
        // Le morceau a été arrêté ou est terminé : on le relance depuis le début.
        if let Err(e) = self.demarrer() {
            self.message = format!("Impossible de lire ce fichier : {e}");
        }
    }

    fn pause(&self) {
        if let Some(telecommande) = &self.telecommande {
            telecommande.mettre_en_pause(true);
        }
    }

    fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.telecommande = None;
    }

    /// Le temps écoulé depuis le début du morceau.
    fn position(&self) -> Duration {
        self.platine_active()
            .map(|t| t.position())
            .unwrap_or_default()
    }

    /// Texte décrivant l'état actuel (en lecture, en pause, arrêté).
    fn etat(&self) -> &'static str {
        match self.platine_active() {
            None => "Arrêté",
            Some(t) if t.en_pause() => "En pause",
            Some(_) => "En lecture",
        }
    }
}

impl LecteurApp {
    /// La barre de progression : temps écoulé / durée totale, et un curseur
    /// qu'on peut faire glisser (avec un bruit de scratch) ou cliquer pour
    /// se déplacer dans le morceau.
    fn barre_de_progression(&mut self, ui: &mut egui::Ui) {
        let duree = self.morceau.as_ref().map(|m| m.duree());
        let duree_s = duree.unwrap_or_default().as_secs_f32();
        // Pendant un glissement, on affiche la position de la souris plutôt que la vraie.
        let mut secondes = self
            .position_glissee
            .unwrap_or_else(|| self.position().as_secs_f32())
            .min(duree_s);

        // Le curseur n'est utilisable que si un morceau est en lecture ou en pause.
        let telecommande = self.platine_active().cloned();

        ui.add_enabled_ui(telecommande.is_some(), |ui| {
            ui.horizontal(|ui| {
                // Le curseur prend toute la largeur, moins la place du texte des temps.
                ui.spacing_mut().slider_width = (ui.available_width() - 90.0).max(50.0);
                // Sans morceau, la plage serait vide (0 à 0) et egui placerait le
                // curseur au milieu : on garde au moins 1 seconde pour qu'il soit à gauche.
                let fin = duree_s.max(1.0);
                let reponse = ui.add(egui::Slider::new(&mut secondes, 0.0..=fin).show_value(false));

                if let Some(telecommande) = &telecommande {
                    if reponse.dragged() {
                        // On glisse : la musique suit la souris (scratch).
                        self.position_glissee = Some(secondes);
                        telecommande.scratcher(secondes);
                    } else if self.position_glissee.take().is_some() || reponse.changed() {
                        // Relâchement après un glissement, ou simple clic : on s'y rend
                        // et la lecture reprend normalement.
                        telecommande.aller_a(secondes);
                    }
                }

                let total = match duree {
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

impl LecteurApp {
    /// Le réglage du volume : un bouton pour couper le son, un curseur et le
    /// pourcentage. Il reste utilisable même sans morceau, pour le préparer.
    fn reglage_du_volume(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let icone = if self.muet || self.volume == 0.0 {
                "🔇"
            } else {
                "🔊"
            };
            let bouton = ui.button(icone).on_hover_text(if self.muet {
                "Remettre le son"
            } else {
                "Couper le son"
            });
            let mut change = false;
            if bouton.clicked() {
                self.muet = !self.muet;
                change = true;
            }

            let reponse = ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false));
            if reponse.changed() {
                // Bouger le curseur remet le son, comme sur la plupart des lecteurs.
                self.muet = false;
                change = true;
            }
            ui.label(format!("{:.0} %", self.volume * 100.0));

            if change {
                self.appliquer_volume();
            }
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

            ui.add_space(8.0);
            self.reglage_du_volume(ui);

            ui.add_space(12.0);
            // Pendant un glissement de la barre, le visualiseur suit la souris.
            let position = self
                .position_glissee
                .unwrap_or_else(|| self.position().as_secs_f32());
            let en_lecture = self.etat() == "En lecture";
            self.visualiseur
                .afficher(ui, self.morceau.as_ref(), position, en_lecture);

            if !self.message.is_empty() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::LIGHT_RED, &self.message);
            }
        });

        // Pendant la lecture (ou en pause, le temps que les barres retombent),
        // on redessine à chaque image de l'écran pour que le visualiseur soit fluide.
        // Sinon, de temps en temps suffit pour voir l'état changer.
        if self.platine_active().is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }
}
