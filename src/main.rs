// En version finale (release), on cache la fenêtre de console noire sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod liste;
mod platine;
mod visualiseur;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use rodio::{OutputStream, OutputStreamHandle, Sink};

use liste::{Action, ListeDeLecture};
use platine::{Morceau, Telecommande};
use visualiseur::Visualiseur;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([520.0, 680.0]),
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
    // Les morceaux à jouer, et lequel est en cours.
    liste: ListeDeLecture,
    // Le son du morceau en cours, décodé en mémoire.
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
            liste: ListeDeLecture::default(),
            morceau: None,
            position_glissee: None,
            volume: 0.5,
            muet: false,
            visualiseur: Visualiseur::new(),
            message,
        }
    }

    /// Ouvre la fenêtre Windows de choix de fichiers (on peut en prendre
    /// plusieurs) et les ajoute à la liste.
    fn ajouter_fichiers(&mut self) {
        let Some(fichiers) = rfd::FileDialog::new()
            .add_filter("Fichiers MP3", &["mp3"])
            .pick_files()
        else {
            return; // l'utilisateur a annulé
        };
        let a_lancer = self.liste.ajouter(fichiers);
        self.jouer_si_rien_en_cours(a_lancer);
    }

    /// Ouvre la fenêtre Windows de choix de dossier et ajoute tous ses MP3.
    fn ajouter_dossier(&mut self) {
        let Some(dossier) = rfd::FileDialog::new().pick_folder() else {
            return; // l'utilisateur a annulé
        };
        match self.liste.ajouter_dossier(&dossier) {
            Some(a_lancer) => self.jouer_si_rien_en_cours(Some(a_lancer)),
            None => self.message = "Aucun fichier MP3 dans ce dossier.".to_string(),
        }
    }

    /// Après un ajout : si rien ne joue, on lance le morceau choisi par la
    /// liste (le premier ajouté, ou l'un d'eux au hasard en lecture aléatoire).
    fn jouer_si_rien_en_cours(&mut self, a_lancer: Option<usize>) {
        if let Some(n) = a_lancer {
            if self.platine_active().is_none() {
                self.jouer(n);
            }
        }
    }

    /// Lance le morceau numéro n de la liste, depuis le début.
    fn jouer(&mut self, n: usize) {
        let Some(chemin) = self.liste.fichier(n).map(Path::to_path_buf) else {
            return;
        };
        self.liste.choisir(n);
        match self.charger(&chemin) {
            Ok(()) => self.message.clear(),
            Err(e) => {
                self.oublier_morceau();
                self.message = format!("Impossible de lire ce fichier : {e}");
            }
        }
    }

    fn suivant(&mut self) {
        match self.liste.suivant() {
            Some(n) => self.jouer(n),
            None => self.stop(), // fin de la liste
        }
    }

    fn precedent(&mut self) {
        if let Some(n) = self.liste.precedent() {
            self.jouer(n);
        }
    }

    /// Arrête la lecture et décharge le morceau (retiré de la liste, ou illisible).
    fn oublier_morceau(&mut self) {
        self.stop();
        self.morceau = None;
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
        // Aucun morceau chargé : on lance celui en cours dans la liste, ou le premier.
        if self.morceau.is_none() {
            self.jouer(self.liste.courant().unwrap_or(0));
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
            ui.horizontal(|ui| {
                if ui.button("📂 Ajouter des MP3…").clicked() {
                    self.ajouter_fichiers();
                }
                if ui.button("📁 Ajouter un dossier…").clicked() {
                    self.ajouter_dossier();
                }
            });

            ui.add_space(8.0);

            let nom = match (&self.morceau, self.liste.courant()) {
                (Some(_), Some(n)) => self
                    .liste
                    .fichier(n)
                    .map(liste::nom_du_fichier)
                    .unwrap_or_default(),
                _ => "Aucun fichier".to_string(),
            };
            ui.label(format!("Morceau : {nom}"));
            ui.label(format!("État : {}", self.etat()));

            ui.add_space(8.0);
            self.barre_de_progression(ui);
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                // Les boutons de lecture ne sont actifs que si la liste contient des morceaux.
                ui.add_enabled_ui(!self.liste.est_vide(), |ui| {
                    if ui.button("⏮").on_hover_text("Morceau précédent").clicked() {
                        self.precedent();
                    }
                    if ui.button("▶ Lecture").clicked() {
                        self.lecture();
                    }
                    if ui.button("⏸ Pause").clicked() {
                        self.pause();
                    }
                    if ui.button("⏹ Stop").clicked() {
                        self.stop();
                    }
                    if ui.button("⏭").on_hover_text("Morceau suivant").clicked() {
                        self.suivant();
                    }
                });
                // La lecture aléatoire se règle à tout moment, même liste vide.
                let aleatoire = self.liste.aleatoire();
                let bouton = ui
                    .selectable_label(aleatoire, "🔀")
                    .on_hover_text(if aleatoire {
                        "Lecture aléatoire : activée"
                    } else {
                        "Lecture aléatoire : désactivée"
                    });
                if bouton.clicked() {
                    self.liste.changer_aleatoire(!aleatoire);
                }
            });

            ui.add_space(8.0);
            self.reglage_du_volume(ui);

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.strong("Liste de lecture");
                if !self.liste.est_vide() && ui.small_button("Vider").clicked() {
                    self.liste.vider();
                    self.oublier_morceau();
                }
            });
            match self.liste.afficher(ui) {
                Some(Action::Jouer(n)) => self.jouer(n),
                Some(Action::Retirer(n)) => {
                    // Si c'était le morceau en cours, on joue celui qui a pris sa
                    // place (le suivant). Au bout de la liste, on s'arrête.
                    if self.liste.retirer(n) {
                        if self.liste.fichier(n).is_some() {
                            self.jouer(n);
                        } else {
                            self.oublier_morceau();
                        }
                    }
                }
                None => {}
            }

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

        // Le morceau en cours vient de se terminer tout seul : on passe au suivant.
        // (Après un Stop, il n'y a plus de lecteur, donc on ne passe pas ici.)
        if self.sink.as_ref().is_some_and(|sink| sink.empty()) {
            self.suivant();
        }

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
