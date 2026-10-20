// En version finale (release), on cache la fenêtre de console noire sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bibliotheque;
mod depot;
mod liste;
mod morceau;
mod platine;
mod playlists;
mod police;
mod raccourcis;
mod tags;
mod visualiseur;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use rodio::{OutputStream, OutputStreamHandle, Sink};

use bibliotheque::Bibliotheque;
use liste::{Action, ListeDeLecture};
use morceau::Morceau;
use platine::Telecommande;
use playlists::{Demande, Playlists};
use raccourcis::Commande;
use tags::TagsConnus;
use visualiseur::Visualiseur;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([920.0, 840.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Lecteur MP3",
        options,
        Box::new(|cc| Ok(Box::new(LecteurApp::new(&cc.egui_ctx)))),
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
    // Tous les MP3 du dossier de musique, à ajouter d'un clic.
    bibliotheque: Bibliotheque,
    // Les listes de lecture enregistrées, en bas de la bibliothèque.
    playlists: Playlists,
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
    // Les tags (titre, artiste, album, durée) des morceaux, lus en arrière-plan.
    tags: TagsConnus,
    // La pochette du morceau en cours, et le fichier dont elle vient (pour
    // ne la relire que quand le morceau change). Rien si le fichier n'en a pas.
    pochette: Option<(PathBuf, Option<egui::TextureHandle>)>,
    // Un message affiché en bas de la fenêtre (erreurs, infos).
    message: String,
}

impl LecteurApp {
    fn new(ctx: &egui::Context) -> Self {
        police::installer(ctx);
        let (sortie, sortie_handle, message) = match OutputStream::try_default() {
            Ok((sortie, handle)) => (Some(sortie), Some(handle), String::new()),
            Err(e) => (None, None, format!("Aucune sortie audio disponible : {e}")),
        };
        Self {
            _sortie: sortie,
            sortie_handle,
            sink: None,
            telecommande: None,
            bibliotheque: Bibliotheque::new(ctx),
            playlists: Playlists::new(),
            liste: ListeDeLecture::default(),
            morceau: None,
            position_glissee: None,
            volume: 0.5,
            muet: false,
            visualiseur: Visualiseur::new(),
            tags: TagsConnus::new(ctx),
            pochette: None,
            message,
        }
    }

    /// Ouvre la fenêtre Windows de choix de fichiers (on peut en prendre
    /// plusieurs) et les ajoute à la liste. Pratique pour un MP3 rangé en
    /// dehors du dossier de musique.
    fn ajouter_fichiers(&mut self) {
        let Some(fichiers) = rfd::FileDialog::new()
            .add_filter("Fichiers MP3", &["mp3"])
            .pick_files()
        else {
            return; // l'utilisateur a annulé
        };
        self.ajouter(fichiers);
    }

    /// Ajoute des morceaux à la fin de la liste, et lance la lecture si rien ne joue.
    fn ajouter(&mut self, fichiers: Vec<PathBuf>) {
        let a_lancer = self.liste.ajouter(fichiers);
        self.jouer_si_rien_en_cours(a_lancer);
    }

    /// Ajoute les fichiers et dossiers lâchés dans la fenêtre, s'il y en a.
    fn recevoir_depot(&mut self, ctx: &egui::Context) {
        let elements = depot::elements_deposes(ctx);
        if elements.is_empty() {
            return;
        }
        let fichiers = depot::mp3_deposes(elements);
        if fichiers.is_empty() {
            self.message = "Aucun fichier MP3 dans ce qui a été déposé.".to_string();
        } else {
            self.ajouter(fichiers);
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
    /// Si ce fichier est illisible (abîmé, déplacé, supprimé), on passe au
    /// suivant au lieu d'arrêter toute la liste.
    fn jouer(&mut self, n: usize) {
        if self.liste.fichier(n).is_none() {
            return;
        }
        match ouvrir_un_lisible(&mut self.liste, n, Morceau::charger) {
            Ok((morceau, saute)) => {
                self.morceau = Some(morceau);
                match self.demarrer() {
                    Ok(()) => self.message = saute,
                    Err(e) => {
                        self.oublier_morceau();
                        self.message = format!("Impossible de lire ce fichier : {e}");
                    }
                }
            }
            Err(message) => {
                self.oublier_morceau();
                self.message = message;
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

    /// Pose le morceau chargé sur une nouvelle platine et la lance.
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
    /// Exécute une commande demandée au clavier.
    fn executer(&mut self, commande: Commande) {
        match commande {
            Commande::LecturePause => match self.platine_active() {
                Some(t) if !t.en_pause() => self.pause(),
                _ if !self.liste.est_vide() => self.lecture(),
                _ => {}
            },
            Commande::Stop => self.stop(),
            Commande::Precedent if !self.liste.est_vide() => self.precedent(),
            Commande::Suivant if !self.liste.est_vide() => self.suivant(),
            Commande::Precedent | Commande::Suivant => {}
            Commande::Deplacer(secondes) => {
                if let (Some(t), Some(morceau)) = (self.platine_active(), &self.morceau) {
                    let fin = morceau.duree().as_secs_f32();
                    let cible = (t.position().as_secs_f32() + secondes).clamp(0.0, fin);
                    t.aller_a(cible);
                }
            }
            Commande::Muet => {
                self.muet = !self.muet;
                self.appliquer_volume();
            }
            Commande::Aleatoire => {
                let aleatoire = self.liste.aleatoire();
                self.liste.changer_aleatoire(!aleatoire);
            }
            Commande::Repetition => {
                let repetition = self.liste.repetition();
                self.liste.changer_repetition(!repetition);
            }
        }
    }

    /// Change le volume (de -1.0 à 1.0), comme si on bougeait le curseur.
    fn changer_volume(&mut self, de: f32) {
        self.volume = (self.volume + de).clamp(0.0, 1.0);
        // Comme le curseur, changer le volume remet le son.
        self.muet = false;
        self.appliquer_volume();
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

/// Choisit le morceau numéro `n` de la liste et l'ouvre avec `ouvrir`. S'il
/// est illisible, on essaie les suivants, chacun une fois au plus (sinon une
/// liste où rien n'est lisible tournerait en rond avec la répétition).
/// Renvoie le morceau ouvert, avec le message du dernier fichier sauté (vide
/// si aucun) ; ou, si rien n'a pu être ouvert, le message à afficher.
fn ouvrir_un_lisible<T>(
    liste: &mut ListeDeLecture,
    n: usize,
    mut ouvrir: impl FnMut(&Path) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<(T, String), String> {
    let mut n = n;
    let mut erreur = None;
    for _ in 0..liste.morceaux().len() {
        let Some(chemin) = liste.fichier(n).map(Path::to_path_buf) else {
            break;
        };
        liste.choisir(n);
        match ouvrir(&chemin) {
            Ok(morceau) => {
                let saute = erreur.map_or(String::new(), |e| format!("{e}, morceau suivant."));
                return Ok((morceau, saute));
            }
            Err(e) => {
                erreur = Some(format!(
                    "Impossible de lire « {} » ({e})",
                    liste::nom_du_fichier(&chemin)
                ));
            }
        }
        match liste.suivant() {
            Some(suivant) => n = suivant,
            None => break,
        }
    }
    Err(erreur.map_or(String::new(), |e| format!("{e}.")))
}

/// Côté de la pochette affichée sous le visualiseur, en points : la plus
/// grande possible dans la place qui reste, entre ces deux tailles.
const POCHETTE_MIN: f32 = 200.0;
const POCHETTE_MAX: f32 = 600.0;

impl LecteurApp {
    /// Lit la pochette du morceau en cours quand il change (rien à faire sinon).
    fn charger_pochette(&mut self, ctx: &egui::Context, en_cours: Option<&Path>) {
        let Some(chemin) = en_cours else {
            self.pochette = None;
            return;
        };
        if self.pochette.as_ref().is_some_and(|(p, _)| p == chemin) {
            return;
        }
        let texture = tags::lire_pochette(chemin)
            .map(|image| ctx.load_texture("pochette", image, egui::TextureOptions::LINEAR));
        self.pochette = Some((chemin.to_path_buf(), texture));
    }

    /// La pochette du morceau en cours, centrée sous le visualiseur.
    fn afficher_pochette(&self, ui: &mut egui::Ui) {
        let Some((_, Some(pochette))) = &self.pochette else {
            return;
        };
        ui.add_space(8.0);
        // On garde de la place pour le message et l'aide des raccourcis en dessous.
        let cote = (ui.available_height() - 40.0)
            .min(ui.available_width())
            .clamp(POCHETTE_MIN, POCHETTE_MAX);
        ui.vertical_centered(|ui| {
            ui.add(egui::Image::new(pochette).fit_to_exact_size(egui::vec2(cote, cote)));
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
        // Les raccourcis clavier passent avant l'interface (voir raccourcis::lire).
        for commande in raccourcis::lire(ctx) {
            self.executer(commande);
        }

        self.recevoir_depot(ctx);

        // À gauche, la bibliothèque. Un clic sur un morceau (ou sur « ➕ tout »)
        // l'ajoute à la liste de lecture. En bas, les playlists.
        let (demandes, demande) = egui::SidePanel::left("bibliotheque")
            .resizable(true)
            .default_width(340.0)
            .width_range(220.0..=600.0)
            .show(ctx, |ui| {
                // Les playlists d'abord, pour qu'elles se réservent le bas du
                // panneau ; la bibliothèque prend la place qui reste au-dessus.
                let demande = egui::TopBottomPanel::bottom("playlists")
                    .resizable(true)
                    .default_height(240.0)
                    .height_range(110.0..=500.0)
                    .show_inside(ui, |ui| {
                        // Le panneau garde la hauteur choisie, même s'il y a
                        // peu de playlists (sinon egui le réduirait à son contenu).
                        ui.set_min_height(ui.available_height());
                        ui.add_space(4.0);
                        self.playlists.afficher(ui, &self.tags)
                    })
                    .inner;
                let noms = self.playlists.noms();
                let dans_la_liste: HashSet<&Path> =
                    self.liste.morceaux().iter().map(PathBuf::as_path).collect();
                (
                    self.bibliotheque
                        .afficher(ui, &dans_la_liste, &self.tags, &noms),
                    demande,
                )
            })
            .inner;
        if !demandes.a_ajouter.is_empty() {
            self.ajouter(demandes.a_ajouter);
        }
        if let Some((nom, morceaux)) = demandes.vers_playlist {
            self.playlists.ajouter_a(&nom, &morceaux);
        }
        match demande {
            // Écouter une playlist : elle remplace la liste de lecture.
            Some(Demande::Charger { fichiers, depart }) => {
                self.liste.vider();
                self.oublier_morceau();
                match depart {
                    // Clic sur un titre de la playlist : on lance celui-là.
                    Some(n) => {
                        self.liste.ajouter(fichiers);
                        self.jouer(n);
                    }
                    None => self.ajouter(fichiers),
                }
            }
            Some(Demande::AjouterALaListe(fichiers)) => self.ajouter(fichiers),
            None => {}
        }

        // La zone de la liste de lecture : la molette y fait défiler la liste,
        // pas changer le volume.
        let mut zone_liste = egui::Rect::NOTHING;
        let panneau = egui::CentralPanel::default().show(ctx, |ui| {
            let en_cours = match (&self.morceau, self.liste.courant()) {
                (Some(_), Some(n)) => self.liste.fichier(n).map(Path::to_path_buf),
                _ => None,
            };
            self.charger_pochette(ui.ctx(), en_cours.as_deref());
            let nom = match &en_cours {
                Some(chemin) => self.tags.libelle(chemin),
                None => "Aucun fichier".to_string(),
            };
            ui.label(format!("Morceau : {nom}"));
            if let Some(album) = en_cours.as_deref().and_then(|c| self.tags.album(c)) {
                ui.label(format!("Album : {album}"));
            }
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
                let repetition = self.liste.repetition();
                let bouton = ui
                    .selectable_label(repetition, "🔁")
                    .on_hover_text(if repetition {
                        "Répétition de la liste : activée"
                    } else {
                        "Répétition de la liste : désactivée"
                    });
                if bouton.clicked() {
                    self.liste.changer_repetition(!repetition);
                }
            });

            ui.add_space(8.0);
            self.reglage_du_volume(ui);

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.strong("Liste de lecture");
                if ui
                    .small_button("📂 Autre fichier…")
                    .on_hover_text("Ajouter un MP3 rangé en dehors de la bibliothèque")
                    .clicked()
                {
                    self.ajouter_fichiers();
                }
                if !self.liste.est_vide() && ui.small_button("Vider").clicked() {
                    self.liste.vider();
                    self.playlists.oublier_la_chargee();
                    self.oublier_morceau();
                }
            });
            let noms = self.playlists.noms();
            let liste = ui.scope(|ui| self.liste.afficher(ui, &self.tags, &noms));
            zone_liste = liste.response.rect;
            match liste.inner {
                // Un clic sur le morceau en cours ne le relance pas depuis le
                // début : il continue, reprend s'il était en pause, ou
                // repart s'il était arrêté.
                Some(Action::Jouer(n)) if self.liste.courant() == Some(n) => self.lecture(),
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
                Some(Action::AjouterAPlaylist { nom, fichier }) => {
                    self.playlists.ajouter_a(&nom, &[fichier]);
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
            self.afficher_pochette(ui);

            if !self.message.is_empty() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::LIGHT_RED, &self.message);
            }

            ui.add_space(8.0);
            ui.weak("⌨ Raccourcis").on_hover_text(raccourcis::AIDE);
        });

        // La molette change le volume dans la partie lecteur de la fenêtre (pas sur la
        // bibliothèque ni sur la liste, où elle sert à faire défiler).
        let souris = ctx.input(|i| i.pointer.hover_pos());
        if souris.is_some_and(|p| panneau.response.rect.contains(p) && !zone_liste.contains(p)) {
            let de = raccourcis::molette(ctx);
            if de != 0.0 {
                self.changer_volume(de);
            }
        }

        depot::dessiner_survol(ctx);
        playlists::dessiner_glissement(ctx);

        // Le morceau en cours vient de se terminer tout seul : on passe au suivant.
        // (Après un Stop, il n'y a plus de lecteur, donc on ne passe pas ici.)
        if self.sink.as_ref().is_some_and(|sink| sink.empty()) {
            self.suivant();
        }

        // Pendant la lecture (ou en pause, le temps que les barres retombent),
        // on redessine à chaque image de l'écran pour que le visualiseur soit fluide.
        // Sinon (en pause, à l'arrêt, ou fenêtre réduite : rien ne bouge à
        // l'écran), de temps en temps suffit pour voir l'état changer et
        // passer au morceau suivant.
        let en_lecture = self.platine_active().is_some_and(|t| !t.en_pause());
        let reduite = ctx.input(|i| i.viewport().minimized == Some(true));
        let anime = en_lecture || !self.visualiseur.au_repos() || self.position_glissee.is_some();
        if anime && !reduite {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une liste de 4 morceaux ; ouvrir réussit seulement pour ceux de `lisibles`.
    fn essayer(
        depart: usize,
        lisibles: &[usize],
        repetition: bool,
    ) -> (Result<(usize, String), String>, Vec<usize>) {
        let mut liste = ListeDeLecture::default();
        liste.ajouter((0..4).map(|i| PathBuf::from(format!("{i}.mp3"))).collect());
        liste.changer_repetition(repetition);
        let mut essayes = Vec::new();
        let resultat = ouvrir_un_lisible(&mut liste, depart, |chemin| {
            let i: usize = chemin
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap();
            essayes.push(i);
            if lisibles.contains(&i) {
                Ok(i)
            } else {
                Err("fichier introuvable".into())
            }
        });
        if let Ok((i, _)) = &resultat {
            assert_eq!(liste.courant(), Some(*i));
        }
        (resultat, essayes)
    }

    #[test]
    fn un_fichier_illisible_passe_au_suivant() {
        let (resultat, essayes) = essayer(1, &[0, 3], false);
        let (ouvert, message) = resultat.unwrap();
        assert_eq!(ouvert, 3);
        assert_eq!(essayes, [1, 2, 3]);
        assert_eq!(
            message,
            "Impossible de lire « 2.mp3 » (fichier introuvable), morceau suivant."
        );
    }

    #[test]
    fn un_fichier_lisible_ne_laisse_pas_de_message() {
        let (resultat, essayes) = essayer(2, &[2], false);
        assert_eq!(resultat.unwrap(), (2, String::new()));
        assert_eq!(essayes, [2]);
    }

    #[test]
    fn au_bout_de_la_liste_on_s_arrete() {
        let (resultat, essayes) = essayer(2, &[0], false);
        assert_eq!(
            resultat.unwrap_err(),
            "Impossible de lire « 3.mp3 » (fichier introuvable)."
        );
        assert_eq!(essayes, [2, 3]);
    }

    #[test]
    fn avec_la_repetition_on_repart_du_debut() {
        let (resultat, essayes) = essayer(2, &[0], true);
        assert_eq!(resultat.unwrap().0, 0);
        assert_eq!(essayes, [2, 3, 0]);
    }

    #[test]
    fn rien_de_lisible_avec_la_repetition_ne_tourne_pas_en_rond() {
        let (resultat, essayes) = essayer(1, &[], true);
        assert!(resultat.is_err());
        assert_eq!(essayes, [1, 2, 3, 0]);
    }
}
