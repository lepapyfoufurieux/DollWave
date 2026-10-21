// En version finale (release), on cache la fenêtre de console noire sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bibliotheque;
mod depot;
mod ecriture_tags;
mod editeur_tags;
mod egalisation;
mod liste;
mod morceau;
mod platine;
mod playlists;
mod police;
mod raccourcis;
mod regroupement;
mod sortie;
mod systeme;
mod tags;
mod visualiseur;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use rodio::Sink;

use bibliotheque::{memoriser, Bibliotheque};
use editeur_tags::EditeurTags;
use egalisation::VolumesConnus;
use liste::{Action, ListeDeLecture};
use morceau::Morceau;
use platine::{Effet, Platine, Telecommande};
use playlists::{Demande, Playlists};
use raccourcis::Commande;
use sortie::{Sortie, Veilleur};
use systeme::{fichier_de_reglage, Boite, Demarrage, ZoneDeNotification};
use tags::TagsConnus;
use visualiseur::Visualiseur;

fn main() -> eframe::Result {
    // Les fichiers donnés au lancement : celui sur lequel on a double-cliqué
    // dans l'explorateur (ou plusieurs). Si un lecteur est déjà ouvert, il
    // les reçoit, et on s'arrête là. Les chemins sont rendus complets : le
    // lecteur déjà ouvert ne tourne pas forcément dans le même dossier.
    let fichiers = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .map(|f| std::path::absolute(&f).unwrap_or(f))
        .collect();
    let boite = match systeme::demarrer(fichiers) {
        Demarrage::Premier(boite) => boite,
        Demarrage::DejaOuvert => return Ok(()),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([920.0, 840.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Lecteur MP3",
        options,
        Box::new(|cc| {
            let mut lecteur = LecteurApp::new(&cc.egui_ctx);
            boite.brancher(&cc.egui_ctx);
            lecteur.boite = boite;
            Ok(Box::new(lecteur))
        }),
    )
}

/// Plusieurs MP3 ouverts d'un coup dans l'explorateur arrivent chacun par son
/// propre lancement, à quelques millisecondes d'écart : ceux qui arrivent
/// dans ce délai sont ajoutés sans couper le premier.
const MEME_SELECTION: Duration = Duration::from_secs(1);

/// Vrai si des fichiers ouverts maintenant font partie de la même sélection
/// que les précédents (voir `MEME_SELECTION`).
fn meme_selection(precedente: Option<Instant>, maintenant: Instant) -> bool {
    precedente.is_some_and(|p| maintenant.saturating_duration_since(p) < MEME_SELECTION)
}

/// Le début du message affiché quand aucune sortie son ne marche.
const SORTIE_INDISPONIBLE: &str = "Sortie audio indisponible";

/// Durée de l'effet platine : le freinage au Stop et à la Pause (un vinyle
/// arrêté à la main), et le démarrage à la Lecture (la vitesse monte de 0 à 1).
const DUREE_PLATINE: Duration = Duration::from_secs(1);
/// Sans l'effet platine : durée du fondu du volume au Stop, à la Pause et à
/// la Lecture (pas de « clic » quand le son se coupe ou revient).
const DUREE_FONDU: Duration = Duration::from_millis(300);
/// Le petit fichier où l'on mémorise si l'effet platine est activé
/// (freinage.txt dans le dossier de réglages, voir `systeme` : « oui » ou « non »).
const REGLAGE_FREINAGE: &str = "freinage.txt";
/// Le petit fichier où l'on mémorise si le scratch s'entend quand on fait
/// glisser la barre de progression (« oui » ou « non »).
const REGLAGE_SCRATCH: &str = "scratch.txt";
/// Le petit fichier où l'on mémorise si le volume égalisé est activé (« oui » ou « non »).
const REGLAGE_EGALISATION: &str = "volume_egalise.txt";
/// Le petit fichier où l'on mémorise si le volume mesuré est écrit dans les
/// MP3 (« oui » ou « non » ; désactivé tant qu'on ne l'a pas coché).
const REGLAGE_ECRITURE_VOLUME: &str = "ecrire_volume.txt";
/// Volume égalisé : facteur appliqué tant que le volume du morceau n'est pas
/// encore connu (mesure en cours). La plupart des morceaux récents sont
/// mixés fort : on les baisse déjà à moitié en attendant.
const FACTEUR_INCONNU: f32 = 0.5;
/// Volume égalisé : en plus du morceau en cours, on mesure à l'avance ce
/// nombre de morceaux qui le suivent dans la liste.
const A_MESURER_A_L_AVANCE: usize = 5;
/// Durée du fondu enchaîné : la fin d'un morceau se mélange au début du
/// suivant pendant ce temps (moins si l'un des deux est très court).
const DUREE_ENCHAINEMENT: Duration = Duration::from_secs(3);
/// Combien de temps avant la fin d'un morceau on ouvre le suivant, pour
/// qu'il soit prêt au moment du fondu enchaîné.
const AVANCE_OUVERTURE: Duration = Duration::from_secs(20);
/// Le petit fichier où l'on mémorise si le fondu enchaîné est activé
/// (« oui » ou « non »).
const REGLAGE_ENCHAINEMENT: &str = "fondu_enchaine.txt";

/// L'état de l'application.
struct LecteurApp {
    // La sortie son (haut-parleurs, casque…). Rien si aucune n'a pu être ouverte.
    sortie: Option<Sortie>,
    // Surveille que la sortie tourne toujours (elle meurt pendant la veille).
    veilleur: Veilleur,
    // Quand regarder si la sortie par défaut de Windows a changé.
    prochaine_verification: Instant,
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
    // Au Stop et à la Pause, le son ralentit-il comme un vinyle qu'on arrête
    // à la main (et accélère-t-il à la Lecture) ?
    freinage: bool,
    // Quand on fait glisser la barre de progression, la musique suit-elle
    // la souris (bruit de scratch) ? Sinon, elle continue et saute au lâcher.
    scratch: bool,
    // Tous les morceaux sonnent-ils aussi fort (volume égalisé) ?
    egalisation: bool,
    // Le volume de chaque morceau, lu dans ses tags ou mesuré en arrière-plan.
    volumes: VolumesConnus,
    // Le volume mesuré est-il aussi écrit dans les MP3 (tags ReplayGain) ?
    ecrire_volume: bool,
    // Le morceau en cours et la taille de la liste lors de la dernière
    // demande de volumes : on ne redemande que quand ça change.
    volumes_demandes: Option<(Option<usize>, usize)>,
    // À la fin d'un morceau, se mélange-t-il au début du suivant (fondu enchaîné) ?
    enchainement: bool,
    // Pendant un fondu enchaîné : le morceau qui finit, en train de s'éteindre.
    // (Le morceau qui commence est dans `sink` et `telecommande`.)
    sortant: Option<(Sink, Telecommande)>,
    // Le morceau suivant, ouvert à l'avance pour le fondu enchaîné.
    a_suivre: Option<Ouverture>,
    // L'image qui bouge avec la musique.
    visualiseur: Visualiseur,
    // Les tags (titre, artiste, album, durée) des morceaux, lus en arrière-plan.
    tags: TagsConnus,
    // L'icône près de l'horloge (sous Windows), où se range la fenêtre réduite.
    zone: ZoneDeNotification,
    // Les fichiers à ouvrir donnés au lancement, puis par les lancements
    // suivants (double-clic sur un MP3 alors que le lecteur est ouvert).
    boite: Boite,
    // Quand le dernier lot de fichiers ouverts depuis l'explorateur est arrivé.
    derniere_ouverture: Option<Instant>,
    // La pochette du morceau en cours, et le fichier dont elle vient (pour
    // ne la relire que quand le morceau change). Rien si le fichier n'en a pas.
    pochette: Option<(PathBuf, Option<egui::TextureHandle>)>,
    // Un message affiché en bas de la fenêtre (erreurs, infos).
    message: String,
    // Les dossiers lâchés dans la fenêtre, en train d'être parcourus.
    depots: depot::Depots,
    // Le morceau qu'on est en train d'ouvrir, en arrière-plan.
    chargement: Option<Chargement>,
    // La pochette du morceau qu'on est en train de lire, en arrière-plan.
    pochette_en_cours: Option<(PathBuf, Receiver<Option<egui::ColorImage>>)>,
    // La fenêtre « Modifier les tags », quand elle est ouverte.
    editeur: Option<EditeurTags>,
    // Pour redessiner la fenêtre quand un travail en arrière-plan est fini.
    ctx: egui::Context,
}

/// Un morceau en train d'être ouvert. Lire tout le fichier prend un moment
/// (surtout pour un long mix) : on le fait en arrière-plan pour que la
/// fenêtre ne se fige pas.
struct Chargement {
    chemin: PathBuf,
    essais: Essais,
    reception: Receiver<Result<Arc<Morceau>, String>>,
    /// Mis à vrai quand on n'attend plus ce morceau (Stop, autre morceau
    /// demandé) : la lecture du fichier s'arrête en route. Sinon, appuyer
    /// longtemps sur → lancerait des dizaines de lectures à la fois.
    abandonne: Arc<AtomicBool>,
    /// On a appuyé sur Pause pendant l'ouverture : le morceau démarrera en pause.
    en_pause: bool,
    /// Le morceau a été lancé par le bouton Lecture : il démarrera en
    /// accélérant (si le freinage est activé).
    en_douceur: bool,
}

/// Le morceau suivant, ouvert en arrière-plan avant la fin du morceau en
/// cours, pour pouvoir les mélanger (fondu enchaîné).
struct Ouverture {
    chemin: PathBuf,
    reception: Receiver<Result<Arc<Morceau>, String>>,
    /// Le morceau, une fois ouvert (ou rien s'il est illisible : on le
    /// laissera alors à l'enchaînement normal, qui passe au suivant).
    pret: Option<Option<Arc<Morceau>>>,
    abandonne: Arc<AtomicBool>,
}

impl Ouverture {
    fn lancer(chemin: PathBuf, ctx: egui::Context) -> Self {
        let (envoi, reception) = mpsc::channel();
        let abandonne = Arc::new(AtomicBool::new(false));
        let (fichier, drapeau) = (chemin.clone(), abandonne.clone());
        std::thread::spawn(move || {
            let resultat = Morceau::charger(&fichier, &drapeau).map_err(|e| e.to_string());
            let _ = envoi.send(resultat);
            ctx.request_repaint();
        });
        Self {
            chemin,
            reception,
            pret: None,
            abandonne,
        }
    }

    /// Le morceau, s'il est ouvert.
    fn morceau(&mut self) -> Option<Arc<Morceau>> {
        if self.pret.is_none() {
            match self.reception.try_recv() {
                Ok(resultat) => self.pret = Some(resultat.ok()),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.pret = Some(None),
            }
        }
        self.pret.clone().flatten()
    }
}

impl Drop for Ouverture {
    fn drop(&mut self) {
        self.abandonne.store(true, Ordering::Relaxed);
    }
}

impl Drop for Chargement {
    fn drop(&mut self) {
        self.abandonne.store(true, Ordering::Relaxed);
    }
}

/// Où en est la lecture.
#[derive(Clone, Copy, PartialEq)]
enum Etat {
    Arrete,
    Chargement,
    Pause,
    Lecture,
}

impl Etat {
    fn texte(self) -> &'static str {
        match self {
            Etat::Arrete => "Arrêté",
            Etat::Chargement => "Chargement…",
            Etat::Pause => "En pause",
            Etat::Lecture => "En lecture",
        }
    }
}

impl LecteurApp {
    fn new(ctx: &egui::Context) -> Self {
        police::installer(ctx);
        let (sortie, message) = match Sortie::ouvrir() {
            Ok(sortie) => (Some(sortie), String::new()),
            Err(e) => (None, format!("Aucune sortie audio disponible : {e}")),
        };
        let maintenant = Instant::now();
        let volumes = VolumesConnus::new(ctx);
        // (Pendant les tests, jamais : ils jouent des fichiers du projet.)
        let ecrire_volume = !cfg!(test) && option_cochee(REGLAGE_ECRITURE_VOLUME);
        volumes.ecrire_dans_les_fichiers(ecrire_volume);
        Self {
            sortie,
            veilleur: Veilleur::new(maintenant),
            prochaine_verification: maintenant + sortie::VERIFICATION,
            sink: None,
            telecommande: None,
            bibliotheque: Bibliotheque::new(ctx),
            playlists: Playlists::new(),
            liste: ListeDeLecture::default(),
            morceau: None,
            position_glissee: None,
            volume: 0.5,
            muet: false,
            freinage: option_memorisee(REGLAGE_FREINAGE),
            scratch: option_memorisee(REGLAGE_SCRATCH),
            egalisation: option_memorisee(REGLAGE_EGALISATION),
            volumes,
            ecrire_volume,
            volumes_demandes: None,
            enchainement: option_memorisee(REGLAGE_ENCHAINEMENT),
            sortant: None,
            a_suivre: None,
            visualiseur: Visualiseur::new(),
            tags: TagsConnus::new(ctx),
            zone: ZoneDeNotification::new(ctx),
            boite: Boite::vide(),
            derniere_ouverture: None,
            pochette: None,
            message,
            depots: depot::Depots::default(),
            chargement: None,
            pochette_en_cours: None,
            editeur: None,
            ctx: ctx.clone(),
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

    /// Ouvre les fichiers reçus de l'explorateur (double-clic sur un MP3) :
    /// ajoutés à la fin de la liste, le premier joue tout de suite, et la
    /// fenêtre revient au premier plan. Un lancement sans fichier ramène
    /// juste la fenêtre.
    fn ouvrir_les_fichiers_recus(&mut self, ctx: &egui::Context) {
        for lot in self.boite.recus() {
            let lancement_sans_fichier = lot.is_empty();
            let fichiers = depot::mp3_deposes(lot);
            let maintenant = Instant::now();
            let meme_selection = meme_selection(self.derniere_ouverture, maintenant);
            if !fichiers.is_empty() {
                self.derniere_ouverture = Some(maintenant);
                let premier = self.liste.morceaux().len();
                if self.liste.ajouter(fichiers).is_some() {
                    if meme_selection {
                        // Ouverts ensemble : le premier joue déjà.
                        self.jouer_si_rien_en_cours(Some(premier));
                    } else {
                        self.jouer(premier);
                    }
                }
            } else if !lancement_sans_fichier {
                self.message = "Aucun fichier MP3 dans ce qui a été ouvert.".to_string();
            }
            self.zone.afficher(ctx);
        }
    }

    /// Ajoute les fichiers et dossiers lâchés dans la fenêtre, s'il y en a.
    /// Les dossiers sont parcourus en arrière-plan : leurs morceaux arrivent
    /// dans la liste quand c'est fini.
    fn recevoir_depot(&mut self, ctx: &egui::Context) {
        let elements = depot::elements_deposes(ctx);
        if !elements.is_empty() {
            self.depots.lancer(ctx, elements);
        }
        if let Some(fichiers) = self.depots.recu() {
            if fichiers.is_empty() {
                self.message = "Aucun fichier MP3 dans ce qui a été déposé.".to_string();
            } else {
                self.ajouter(fichiers);
            }
        }
    }

    /// Après un ajout : si rien ne joue, on lance le morceau choisi par la
    /// liste (le premier ajouté, ou l'un d'eux au hasard en lecture aléatoire).
    fn jouer_si_rien_en_cours(&mut self, a_lancer: Option<usize>) {
        if let Some(n) = a_lancer {
            if self.etat() == Etat::Arrete {
                self.jouer(n);
            }
        }
    }

    /// Lance le morceau numéro n de la liste, depuis le début. Le fichier
    /// est ouvert en arrière-plan (voir `suivre_le_chargement`) ; s'il est
    /// illisible (abîmé, déplacé, supprimé), on passe au suivant au lieu
    /// d'arrêter toute la liste.
    fn jouer(&mut self, n: usize) {
        if self.liste.fichier(n).is_none() {
            return;
        }
        self.oublier_morceau();
        let essais = Essais::pour(&self.liste);
        self.essayer(n, essais);
    }

    /// Commence à ouvrir le morceau numéro n, en arrière-plan.
    fn essayer(&mut self, n: usize, essais: Essais) {
        let Some(chemin) = self.liste.fichier(n).map(Path::to_path_buf) else {
            return;
        };
        self.liste.choisir(n);
        let en_pause = self.chargement.as_ref().is_some_and(|c| c.en_pause);
        let en_douceur = self.chargement.as_ref().is_some_and(|c| c.en_douceur);
        let (envoi, reception) = mpsc::channel();
        let abandonne = Arc::new(AtomicBool::new(false));
        let (fichier, ctx, drapeau) = (chemin.clone(), self.ctx.clone(), abandonne.clone());
        std::thread::spawn(move || {
            let resultat = Morceau::charger(&fichier, &drapeau).map_err(|e| e.to_string());
            let _ = envoi.send(resultat);
            ctx.request_repaint();
        });
        self.chargement = Some(Chargement {
            chemin,
            essais,
            reception,
            abandonne,
            en_pause,
            en_douceur,
        });
    }

    /// Le morceau qu'on ouvrait est-il prêt ? On le lance, ou on essaie le
    /// suivant s'il est illisible.
    fn suivre_le_chargement(&mut self) {
        let Some(chargement) = &self.chargement else {
            return;
        };
        let resultat = match chargement.reception.try_recv() {
            Ok(resultat) => resultat,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("lecture interrompue".to_string()),
        };
        let Some(chargement) = self.chargement.take() else {
            return;
        };
        let chemin = chargement.chemin.clone();
        let en_pause = chargement.en_pause;
        let en_douceur = chargement.en_douceur;
        let mut essais = chargement.essais.clone();
        match resultat {
            Ok(morceau) => {
                self.morceau = Some(morceau);
                match self.demarrer(en_douceur) {
                    Ok(()) => {
                        if let Some(t) = self.telecommande.as_ref().filter(|_| en_pause) {
                            t.mettre_en_pause(true);
                        }
                        self.message = essais.message_si_lu();
                    }
                    Err(e) => {
                        self.oublier_morceau();
                        self.message = format!("Impossible de lancer la lecture : {e}");
                    }
                }
            }
            Err(e) => match essais.echec(&mut self.liste, &chemin, &e) {
                Some(suivant) => {
                    // On garde la pause demandée pour le morceau suivant.
                    self.chargement = Some(chargement);
                    self.essayer(suivant, essais);
                }
                None => {
                    self.oublier_morceau();
                    self.message = essais.message_si_rien_de_lu();
                }
            },
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

    /// Pose le morceau chargé sur une nouvelle platine et la lance, en
    /// partant en douceur depuis l'arrêt si `en_douceur` (effet platine ou
    /// fondu, voir `effet`).
    fn demarrer(&mut self, en_douceur: bool) -> Result<(), Box<dyn std::error::Error>> {
        // On arrête l'ancien morceau s'il y en avait un.
        self.stop();

        let morceau = self.morceau.clone().ok_or("aucun morceau chargé")?;
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau);
        if en_douceur {
            // Avant de lancer : sinon le début partirait d'un coup.
            let (effet, duree) = self.effet();
            telecommande.demarrer_en_douceur(effet, duree);
            platine = platine.depuis_l_arret();
        }
        self.lancer(telecommande, platine)
    }

    /// Envoie une platine vers la sortie son.
    fn lancer(
        &mut self,
        telecommande: Telecommande,
        platine: Platine,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let sortie = self.sortie.as_ref().ok_or("pas de sortie audio")?;
        let sink = Sink::try_new(&sortie.handle)?;
        // Le volume égalisé est réglé avant le premier son.
        telecommande.egaliser(self.facteur_egalisation());
        sink.append(platine);
        self.sink = Some(sink);
        self.appliquer_volume();
        self.telecommande = Some(telecommande);
        Ok(())
    }

    /// Vérifie que la sortie son marche toujours (la veille ou un casque
    /// débranché la tuent sans prévenir), et qu'elle est bien celle choisie
    /// dans Windows. Sinon, on en ouvre une nouvelle.
    fn surveiller_sortie(&mut self, maintenant: Instant) {
        let muette = self
            .sortie
            .as_ref()
            .is_some_and(|sortie| self.veilleur.muette(sortie.battements(), maintenant));
        let mut changee = false;
        if maintenant >= self.prochaine_verification {
            self.prochaine_verification = maintenant + sortie::VERIFICATION;
            changee = match (&self.sortie, sortie::nom_par_defaut()) {
                (None, _) => true, // pas de sortie : on réessaie
                (Some(sortie), Some(nom)) => sortie.nom.as_ref() != Some(&nom),
                (Some(_), None) => false, // rien d'autre vers quoi aller
            };
        }
        if !muette && !changee {
            return;
        }
        match Sortie::ouvrir() {
            Ok(nouvelle) => self.passer_sur(nouvelle, maintenant),
            Err(e) => {
                // On réessaiera dans un moment (pas à chaque image).
                self.veilleur = Veilleur::new(maintenant);
                self.message = format!("{SORTIE_INDISPONIBLE} ({e}), nouvel essai…");
            }
        }
    }

    /// Remplace la sortie son. Le morceau en cours reprend au même endroit
    /// sur la nouvelle, en pause s'il l'était.
    fn passer_sur(&mut self, sortie: Sortie, maintenant: Instant) {
        // Un morceau en train de freiner est déjà arrêté : on ne le reprend pas.
        let reprise = self
            .platine_active()
            .filter(|t| !t.freine())
            .map(Telecommande::reprise);
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.telecommande = None;
        // Un morceau en train de s'éteindre (fondu enchaîné) : on le coupe.
        if let Some((sink, _)) = self.sortant.take() {
            sink.stop();
        }
        // Le son est revenu : on efface le message de panne, s'il y en avait un.
        if self.message.starts_with(SORTIE_INDISPONIBLE) {
            self.message.clear();
        }
        self.sortie = Some(sortie);
        self.veilleur = Veilleur::new(maintenant);
        if let Some((telecommande, platine)) = reprise {
            if let Err(e) = self.lancer(telecommande, platine) {
                self.message = format!("Impossible de reprendre la lecture : {e}");
            }
        }
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
            if let Some((sortant, _)) = &self.sortant {
                sortant.set_volume(volume);
            }
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
        // Le morceau est en train de s'ouvrir : il se lancera tout seul.
        if let Some(chargement) = &mut self.chargement {
            chargement.en_pause = false;
            return;
        }
        // Le morceau est encore en cours : on reprend là où on en était, en
        // douceur (le disque se relance, ou le volume remonte).
        // (S'il s'arrête après un Stop, il est arrêté : on le relance plus bas.)
        let effet = self.effet();
        if let Some(telecommande) = self.platine_active().filter(|t| !t.freine()) {
            // (Pendant un fondu enchaîné, le morceau qui s'éteint repart aussi.)
            // (Déjà en lecture, ou en train de repartir : rien à faire.)
            let sortant = self.sortant.as_ref().map(|(_, t)| t);
            for t in std::iter::once(telecommande).chain(sortant) {
                if t.en_pause() {
                    t.demarrer_en_douceur(effet.0, effet.1);
                }
            }
            return;
        }
        // Aucun morceau chargé : on lance celui en cours dans la liste, ou le premier.
        if self.morceau.is_none() {
            self.jouer(self.liste.courant().unwrap_or(0));
            if let Some(chargement) = &mut self.chargement {
                chargement.en_douceur = true;
            }
            return;
        }
        // Le morceau a été arrêté ou est terminé : on le relance depuis le début.
        if let Err(e) = self.demarrer(true) {
            self.message = format!("Impossible de lancer la lecture : {e}");
        }
    }

    fn pause(&mut self) {
        if let Some(chargement) = &mut self.chargement {
            chargement.en_pause = true; // il démarrera en pause
        }
        // (Pendant un fondu enchaîné, le morceau qui s'éteint se met aussi en pause.)
        let sortant = self.sortant.as_ref().map(|(_, t)| t);
        for telecommande in self.telecommande.iter().chain(sortant) {
            // (Déjà en train de s'arrêter après un Stop : on le laisse finir.)
            if !telecommande.freine() {
                // Le disque ralentit (ou le volume baisse), puis se fige au
                // même endroit.
                let (effet, duree) = self.effet();
                telecommande.freiner_jusqu_a_la_pause(effet, duree);
            }
        }
    }

    /// Le bouton Stop et la touche S : si un morceau joue, il s'arrête en
    /// douceur (voir `Telecommande::freiner`) ; sinon (en pause, ou au
    /// second appui), on coupe net.
    fn arreter(&mut self) {
        let (effet, duree) = self.effet();
        match self.platine_active() {
            Some(t) if !t.en_pause() && !t.freine() => {
                t.freiner(effet, duree);
                // Pendant un fondu enchaîné, le morceau qui s'éteint s'arrête aussi.
                if let Some((_, sortant)) = &self.sortant {
                    sortant.freiner(effet, duree);
                }
            }
            _ => self.stop(),
        }
    }

    /// Comment le son s'arrête et repart : effet platine (option du menu
    /// Options), ou simple fondu du volume.
    fn effet(&self) -> (Effet, Duration) {
        if self.freinage {
            (Effet::Platine, DUREE_PLATINE)
        } else {
            (Effet::Fondu, DUREE_FONDU)
        }
    }

    /// On fait glisser la barre de progression jusqu'à `secondes` : la
    /// musique suit la souris (scratch), ou continue sans bouger si le son du
    /// scratch est coupé. Dans les deux cas, elle saute là au lâcher.
    fn glisser(&mut self, telecommande: &Telecommande, secondes: f32) {
        self.position_glissee = Some(secondes);
        if self.scratch {
            telecommande.scratcher(secondes);
        }
    }

    /// Le menu Options : effet platine et son du scratch, mémorisés.
    fn menu_options(&mut self, ui: &mut egui::Ui) {
        if ui
            .checkbox(&mut self.freinage, "Effet platine")
            .on_hover_text(
                "Au Stop et à la Pause, le son ralentit et descend dans les graves, \
                 comme un vinyle qu'on arrête à la main ; à la Lecture, il accélère. \
                 Sinon, le volume baisse ou monte en une fraction de seconde.",
            )
            .changed()
        {
            memoriser_option(REGLAGE_FREINAGE, self.freinage);
        }
        if ui
            .checkbox(&mut self.scratch, "Son du scratch")
            .on_hover_text(
                "Quand on fait glisser la barre de progression, la musique suit la \
                 souris comme un vinyle. Sinon, elle continue et saute au lâcher.",
            )
            .changed()
        {
            memoriser_option(REGLAGE_SCRATCH, self.scratch);
        }
        if ui
            .checkbox(&mut self.egalisation, "Volume égalisé")
            .on_hover_text(
                "Tous les morceaux sonnent aussi fort, d'un album à l'autre. Le volume \
                 de chaque morceau vient de ses tags ReplayGain s'il en a ; sinon, le \
                 lecteur le mesure une fois et s'en souvient. Le curseur de volume \
                 s'applique par-dessus.",
            )
            .changed()
        {
            memoriser_option(REGLAGE_EGALISATION, self.egalisation);
            self.volumes_demandes = None;
        }
        ui.indent("ecrire_volume", |ui| {
            ui.add_enabled_ui(self.egalisation, |ui| {
                if ui
                    .checkbox(&mut self.ecrire_volume, "Écrire le volume dans les MP3")
                    .on_hover_text(
                        "Le volume mesuré par le lecteur est aussi écrit dans le fichier \
                         (tags ReplayGain), pour que d'autres lecteurs (téléphone, \
                         foobar2000, MusicBee…) en profitent. Seuls les morceaux \
                         écoutés (et les suivants de la liste) sont concernés ; les \
                         tags ReplayGain déjà présents ne sont jamais changés.",
                    )
                    .changed()
                {
                    memoriser_option(REGLAGE_ECRITURE_VOLUME, self.ecrire_volume);
                    self.volumes.ecrire_dans_les_fichiers(self.ecrire_volume);
                    self.volumes_demandes = None;
                }
            });
        });
        if ui
            .checkbox(&mut self.enchainement, "Fondu enchaîné")
            .on_hover_text(
                "À la fin d'un morceau, il se mélange au début du suivant pendant \
                 quelques secondes, comme à la radio",
            )
            .changed()
        {
            memoriser_option(REGLAGE_ENCHAINEMENT, self.enchainement);
            if !self.enchainement {
                self.a_suivre = None;
            }
        }
    }

    /// Le facteur du volume égalisé pour le morceau en cours : 1 si
    /// l'option est décochée, `FACTEUR_INCONNU` tant qu'il n'est pas mesuré.
    fn facteur_egalisation(&self) -> f32 {
        if !self.egalisation {
            return 1.0;
        }
        self.liste
            .courant()
            .and_then(|n| self.liste.fichier(n))
            .and_then(|chemin| self.volumes.gain(chemin))
            .unwrap_or(FACTEUR_INCONNU)
    }

    /// Demande le volume du morceau en cours (en urgence) et des suivants,
    /// et l'applique à la platine dès qu'il est connu.
    fn suivre_l_egalisation(&mut self) {
        let demande = (self.liste.courant(), self.liste.morceaux().len());
        if self.egalisation && self.volumes_demandes != Some(demande) {
            self.volumes_demandes = Some(demande);
            if let Some(courant) = demande.0 {
                // Le vrai morceau suivant d'abord (en aléatoire, celui déjà
                // tiré au sort), puis ceux qui suivent dans la liste.
                let a_suivre = self.liste.a_suivre();
                let morceaux = self.liste.morceaux();
                let ensuite = (1..=A_MESURER_A_L_AVANCE.min(morceaux.len().saturating_sub(1)))
                    .map(|ecart| (courant + ecart) % morceaux.len());
                let numeros = std::iter::once(courant).chain(a_suivre).chain(ensuite);
                self.volumes
                    .demander(numeros.map(|n| morceaux[n].as_path()));
            }
        }
        if let Some(telecommande) = &self.telecommande {
            telecommande.egaliser(self.facteur_egalisation());
        }
    }

    /// Arrête la lecture (et le morceau qu'on était peut-être en train d'ouvrir).
    fn stop(&mut self) {
        self.chargement = None;
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.telecommande = None;
        // Le morceau qui s'éteignait (fondu enchaîné) s'arrête net aussi :
        // ⏭, ⏮ ou un clic sur un titre changent de morceau tout de suite.
        if let Some((sink, _)) = self.sortant.take() {
            sink.stop();
        }
        self.a_suivre = None;
    }

    /// Le fondu enchaîné : un peu avant la fin du morceau, on ouvre le
    /// suivant en arrière-plan ; quelques secondes avant la fin, on le lance
    /// en montant le son, pendant que celui qui finit s'éteint. La barre, le
    /// titre et la liste passent tout de suite au nouveau morceau.
    fn preparer_l_enchainement(&mut self) {
        // Le morceau qui s'éteignait a fini de s'éteindre.
        if self.sortant.as_ref().is_some_and(|(sink, _)| sink.empty()) {
            self.sortant = None;
        }
        let restant = match (self.platine_active(), &self.morceau) {
            (Some(t), Some(morceau)) if self.enchainement && self.chargement.is_none() => {
                Some(morceau.duree().saturating_sub(t.position()))
            }
            _ => None,
        };
        let Some(restant) = restant.filter(|&r| r <= AVANCE_OUVERTURE) else {
            // Pas de fondu en vue (ou on est revenu en arrière dans le morceau).
            self.a_suivre = None;
            return;
        };
        // Le morceau suivant peut changer d'ici la fin (liste modifiée,
        // « Lire ensuite »…) : on vérifie à chaque fois que c'est le bon.
        let Some(n) = self.liste.a_suivre() else {
            self.a_suivre = None; // fin de la liste : la musique s'arrêtera
            return;
        };
        let Some(chemin) = self.liste.fichier(n).map(Path::to_path_buf) else {
            return;
        };
        if self.a_suivre.as_ref().is_none_or(|o| o.chemin != chemin) {
            self.a_suivre = Some(Ouverture::lancer(chemin, self.ctx.clone()));
        }
        // On ne commence pas le fondu pendant une pause, un Stop ou un scratch.
        let Some(t) = self.platine_active() else {
            return;
        };
        if t.en_pause() || t.freine() || self.position_glissee.is_some() {
            return;
        }
        let Some(suivant) = self.a_suivre.as_mut().and_then(Ouverture::morceau) else {
            return;
        };
        let duree = duree_de_l_enchainement(
            self.morceau.as_ref().map_or(Duration::ZERO, |m| m.duree()),
            suivant.duree(),
        );
        if restant <= duree && !duree.is_zero() {
            self.enchainer(n, suivant, duree.min(restant));
        }
    }

    /// Lance le morceau numéro n (déjà ouvert) en fondu enchaîné : il monte
    /// pendant `duree`, et celui en cours s'éteint pendant le même temps.
    fn enchainer(&mut self, n: usize, morceau: Arc<Morceau>, duree: Duration) {
        self.a_suivre = None;
        if let Some((sink, _)) = self.sortant.take() {
            sink.stop(); // (un fondu précédent, déjà presque fini)
        }
        if let (Some(sink), Some(telecommande)) = (self.sink.take(), self.telecommande.take()) {
            telecommande.fondre_en_sortie(duree);
            self.sortant = Some((sink, telecommande));
        }
        self.liste.suivant();
        self.liste.choisir(n);
        self.morceau = Some(morceau.clone());
        let (telecommande, platine) = Telecommande::nouvelle(morceau);
        telecommande.fondre_en_entree(duree);
        if let Err(e) = self.lancer(telecommande, platine) {
            self.message = format!("Impossible de lancer la lecture : {e}");
        }
        // (Le morceau qui s'éteint garde le même volume que le nouveau.)
        self.appliquer_volume();
    }

    /// Le temps écoulé depuis le début du morceau.
    fn position(&self) -> Duration {
        self.platine_active()
            .map(|t| t.position())
            .unwrap_or_default()
    }

    /// Le morceau en cours vient de se terminer tout seul : on passe au
    /// suivant. Sauf si c'est la fin d'un freinage : on reste arrêté.
    /// (Après un Stop sans freinage, il n'y a plus de lecteur : rien à faire.)
    fn suivre_la_fin(&mut self) {
        if self.sink.as_ref().is_some_and(|sink| sink.empty()) {
            if self.telecommande.as_ref().is_some_and(|t| t.freine()) {
                self.stop();
            } else {
                self.suivant();
            }
        }
    }

    /// Où en est la lecture (en lecture, en pause, arrêté…).
    fn etat(&self) -> Etat {
        if self.chargement.is_some() {
            return Etat::Chargement;
        }
        match self.platine_active() {
            None => Etat::Arrete,
            Some(t) if t.freine() => Etat::Arrete, // Stop pendant le freinage
            Some(t) if t.en_pause() => Etat::Pause,
            Some(_) => Etat::Lecture,
        }
    }
}

impl LecteurApp {
    /// Exécute une commande demandée au clavier.
    fn executer(&mut self, commande: Commande) {
        match commande {
            Commande::LecturePause => match self.platine_active() {
                None if self.chargement.as_ref().is_some_and(|c| !c.en_pause) => self.pause(),
                Some(t) if !t.en_pause() && !t.freine() => self.pause(),
                _ if !self.liste.est_vide() => self.lecture(),
                _ => {}
            },
            Commande::Stop => self.arreter(),
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
        // (Pas pendant le freinage : le morceau est déjà arrêté.)
        let telecommande = self.platine_active().filter(|t| !t.freine()).cloned();

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
                        self.glisser(telecommande, secondes);
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
    /// La fenêtre « Modifier les tags », si elle est ouverte. Une fois les
    /// tags enregistrés, le fichier est relu : la liste, la bibliothèque,
    /// les vues Artistes/Albums et la pochette se mettent à jour.
    fn afficher_editeur(&mut self, ctx: &egui::Context) {
        let Some(editeur) = &mut self.editeur else {
            return;
        };
        let retour = editeur.afficher(ctx);
        for chemin in &retour.a_relire {
            self.tags.relire(chemin);
        }
        let touche = |chemin: &PathBuf| retour.a_relire.contains(chemin);
        if self.pochette.as_ref().is_some_and(|(p, _)| touche(p))
            || self
                .pochette_en_cours
                .as_ref()
                .is_some_and(|(p, _)| touche(p))
        {
            self.pochette = None;
            self.pochette_en_cours = None;
        }
        if retour.fermer {
            self.editeur = None;
        }
    }

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

/// Les essais pour trouver un morceau lisible, à partir de celui demandé :
/// chaque morceau est essayé une fois au plus (sinon une liste où rien
/// n'est lisible tournerait en rond avec la répétition).
#[derive(Clone)]
struct Essais {
    restants: usize,
    /// Le dernier fichier sauté, et pourquoi.
    erreur: Option<String>,
}

impl Essais {
    fn pour(liste: &ListeDeLecture) -> Self {
        Self {
            restants: liste.morceaux().len(),
            erreur: None,
        }
    }

    /// Le morceau `chemin` n'a pas pu être ouvert : renvoie le numéro du
    /// suivant à essayer, ou rien s'il faut abandonner.
    fn echec(&mut self, liste: &mut ListeDeLecture, chemin: &Path, erreur: &str) -> Option<usize> {
        self.erreur = Some(format!(
            "Impossible de lire « {} » ({erreur})",
            liste::nom_du_fichier(chemin)
        ));
        self.restants = self.restants.saturating_sub(1);
        if self.restants == 0 {
            return None;
        }
        liste.suivant()
    }

    /// Le message à afficher une fois un morceau ouvert (vide si aucun n'a été sauté).
    fn message_si_lu(&self) -> String {
        self.erreur
            .as_ref()
            .map_or(String::new(), |e| format!("{e}, morceau suivant."))
    }

    /// Le message à afficher si rien n'a pu être ouvert.
    fn message_si_rien_de_lu(&self) -> String {
        self.erreur
            .as_ref()
            .map_or(String::new(), |e| format!("{e}."))
    }
}

/// Comme l'appli, mais d'un coup (sans arrière-plan) : choisit le morceau
/// numéro `n`, l'ouvre avec `ouvrir`, et passe aux suivants s'il est
/// illisible. Pour tester `Essais`.
#[cfg(test)]
fn ouvrir_un_lisible<T>(
    liste: &mut ListeDeLecture,
    n: usize,
    mut ouvrir: impl FnMut(&Path) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<(T, String), String> {
    let mut essais = Essais::pour(liste);
    let mut n = n;
    loop {
        let Some(chemin) = liste.fichier(n).map(Path::to_path_buf) else {
            return Err(essais.message_si_rien_de_lu());
        };
        liste.choisir(n);
        match ouvrir(&chemin) {
            Ok(morceau) => return Ok((morceau, essais.message_si_lu())),
            Err(e) => match essais.echec(liste, &chemin, &e.to_string()) {
                Some(suivant) => n = suivant,
                None => return Err(essais.message_si_rien_de_lu()),
            },
        }
    }
}

/// Côté de la pochette affichée sous le visualiseur, en points : la plus
/// grande possible dans la place qui reste, entre ces deux tailles.
const POCHETTE_MIN: f32 = 200.0;
const POCHETTE_MAX: f32 = 600.0;

impl LecteurApp {
    /// Lit la pochette du morceau en cours quand il change (rien à faire
    /// sinon). Décoder l'image prend un moment : on le fait en arrière-plan.
    fn charger_pochette(&mut self, ctx: &egui::Context, en_cours: Option<&Path>) {
        let Some(chemin) = en_cours else {
            self.pochette = None;
            self.pochette_en_cours = None;
            return;
        };
        // La pochette demandée est prête : on l'envoie à la carte graphique.
        if let Some((demandee, reception)) = &self.pochette_en_cours {
            if demandee == chemin {
                match reception.try_recv() {
                    Ok(image) => {
                        let texture = image.map(|image| {
                            ctx.load_texture("pochette", image, egui::TextureOptions::LINEAR)
                        });
                        self.pochette = Some((chemin.to_path_buf(), texture));
                        self.pochette_en_cours = None;
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        // La lecture de la pochette a échoué : pas de pochette,
                        // sans réessayer à chaque image.
                        self.pochette = Some((chemin.to_path_buf(), None));
                        self.pochette_en_cours = None;
                    }
                }
                return;
            }
        }
        if self.pochette.as_ref().is_some_and(|(p, _)| p == chemin) {
            return;
        }
        // Nouveau morceau : on oublie l'ancienne pochette et on lit la sienne.
        self.pochette = None;
        let (envoi, reception) = mpsc::channel();
        let (fichier, ctx) = (chemin.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let _ = envoi.send(tags::lire_pochette(&fichier));
            ctx.request_repaint();
        });
        self.pochette_en_cours = Some((chemin.to_path_buf(), reception));
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

/// Une option du menu Options est-elle activée ? Oui, sauf si on l'a
/// désactivée (« non » dans son fichier de réglage).
fn option_memorisee(fichier: &str) -> bool {
    let texte = fichier_de_reglage(fichier).and_then(|f| std::fs::read_to_string(f).ok());
    option_depuis(texte.as_deref())
}

/// Une option désactivée par défaut est-elle cochée ? Seulement si on l'a cochée.
fn option_cochee(fichier: &str) -> bool {
    let texte = fichier_de_reglage(fichier).and_then(|f| std::fs::read_to_string(f).ok());
    option_cochee_depuis(texte.as_deref())
}

/// Lit le fichier d'une option désactivée par défaut.
fn option_cochee_depuis(texte: Option<&str>) -> bool {
    texte.is_some_and(|t| t.trim() == "oui")
}

/// Lit le fichier d'une option : activée par défaut (pas de fichier, ou
/// fichier illisible).
fn option_depuis(texte: Option<&str>) -> bool {
    texte.is_none_or(|t| t.trim() != "non")
}

/// Mémorise une option du menu Options.
fn memoriser_option(fichier: &str, activee: bool) {
    memoriser(fichier, if activee { "oui" } else { "non" });
}

/// Durée du fondu enchaîné entre un morceau et le suivant : `DUREE_ENCHAINEMENT`,
/// sauf si l'un des deux est très court (au plus la moitié de chacun).
fn duree_de_l_enchainement(finit: Duration, commence: Duration) -> Duration {
    DUREE_ENCHAINEMENT.min(finit / 2).min(commence / 2)
}

/// Écrit une durée sous la forme « minutes:secondes », par exemple 3:07.
fn format_temps(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

impl eframe::App for LecteurApp {
    // Appelée à chaque image : on (re)dessine toute l'interface.
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Les raccourcis clavier passent avant l'interface (voir raccourcis::lire),
        // tout comme le menu de l'icône près de l'horloge.
        for commande in raccourcis::lire(ctx) {
            self.executer(commande);
        }
        self.ouvrir_les_fichiers_recus(ctx);
        for commande in self.zone.surveiller(ctx, frame) {
            self.executer(commande);
        }

        self.recevoir_depot(ctx);
        self.suivre_le_chargement();
        self.suivre_l_egalisation();
        self.preparer_l_enchainement();

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
        if let Some(chemins) = demandes.tags_a_modifier {
            self.editeur = Some(EditeurTags::ouvrir(ctx, chemins, &self.tags));
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
            // Pendant l'ouverture, on montre déjà le nom du morceau.
            let en_cours = en_cours.or_else(|| self.chargement.as_ref().map(|c| c.chemin.clone()));
            let nom = match &en_cours {
                Some(chemin) => self.tags.libelle(chemin),
                None => "Aucun fichier".to_string(),
            };
            self.zone
                .infobulle(en_cours.is_some().then_some(nom.as_str()));
            ui.label(format!("Morceau : {nom}"));
            if let Some(album) = en_cours.as_deref().and_then(|c| self.tags.album(c)) {
                ui.label(format!("Album : {album}"));
            }
            ui.label(format!("État : {}", self.etat().texte()));

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
                        self.arreter();
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
                let compteur = self.liste.compteur(&self.tags);
                if !compteur.is_empty() {
                    ui.weak(compteur);
                }
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
                    // Si c'était le morceau en cours et qu'il jouait, on passe au
                    // suivant (au hasard en lecture aléatoire). En pause ou à
                    // l'arrêt, on s'arrête simplement.
                    let jouait = matches!(self.etat(), Etat::Lecture | Etat::Chargement);
                    if self.liste.retirer(n) {
                        match jouait.then(|| self.liste.apres_retrait(n)).flatten() {
                            Some(suivant) => self.jouer(suivant),
                            None => self.oublier_morceau(),
                        }
                    }
                }
                Some(Action::AjouterAPlaylist { nom, fichier }) => {
                    self.playlists.ajouter_a(&nom, &[fichier]);
                }
                Some(Action::ModifierLesTags(fichier)) => {
                    self.editeur = Some(EditeurTags::ouvrir(ui.ctx(), vec![fichier], &self.tags));
                }
                None => {}
            }

            ui.add_space(12.0);
            // Pendant un glissement de la barre, le visualiseur suit la souris.
            let position = self
                .position_glissee
                .unwrap_or_else(|| self.position().as_secs_f32());
            // (Pendant le freinage aussi : le son continue de sortir.)
            let en_lecture = self.platine_active().is_some_and(Telecommande::tourne);
            self.visualiseur
                .afficher(ui, self.morceau.as_ref(), position, en_lecture);
            self.afficher_pochette(ui);

            if !self.message.is_empty() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::LIGHT_RED, &self.message);
            }
            if self.depots.en_cours() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Lecture des fichiers déposés…");
                });
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.menu_button("⚙ Options", |ui| self.menu_options(ui));
                ui.weak("⌨ Raccourcis").on_hover_text(raccourcis::AIDE);
            });
            // La croix de la fenêtre range le lecteur près de l'horloge
            // (sous Windows) : ce bouton-ci le ferme vraiment.
            if ui.button("Quitter").clicked() {
                self.zone.quitter(ctx);
            }
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

        self.afficher_editeur(ctx);
        depot::dessiner_survol(ctx);
        playlists::dessiner_glissement(ctx);

        self.surveiller_sortie(Instant::now());

        self.suivre_la_fin();

        // Pendant la lecture (ou en pause, le temps que les barres retombent),
        // on redessine à chaque image de l'écran pour que le visualiseur soit fluide.
        // Sinon (en pause, à l'arrêt, ou fenêtre réduite : rien ne bouge à
        // l'écran), de temps en temps suffit pour voir l'état changer et
        // passer au morceau suivant.
        let en_lecture = self.platine_active().is_some_and(Telecommande::tourne);
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

    /// Les tests qui créent un lecteur entier (avec sa sortie son) passent
    /// un par un.
    static UN_LECTEUR_A_LA_FOIS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// À garder tant que le lecteur du test existe.
    fn mon_tour() -> std::sync::MutexGuard<'static, ()> {
        garder_le_son_de_windows_en_vie();
        UN_LECTEUR_A_LA_FOIS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Sous Windows, cpal (la bibliothèque du son) crée une seule fois un
    /// objet Windows qui liste les sorties son, dans le fil qui s'en sert en
    /// premier. Quand ce fil se termine, l'objet meurt avec lui, et le fil
    /// suivant qui ouvre une sortie plante (STATUS_ACCESS_VIOLATION). Dans
    /// l'appli, c'est toujours le fil de la fenêtre, qui vit jusqu'au bout ;
    /// mais chaque test a son propre fil. On crée donc l'objet une fois pour
    /// toutes dans un fil qui ne se termine jamais.
    fn garder_le_son_de_windows_en_vie() {
        static UNE_FOIS: std::sync::Once = std::sync::Once::new();
        UNE_FOIS.call_once(|| {
            let (pret, attendre) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sortie::nom_par_defaut();
                let _ = pret.send(());
                loop {
                    std::thread::park();
                }
            });
            let _ = attendre.recv();
        });
    }

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
    fn des_mp3_ouverts_ensemble_ne_se_coupent_pas() {
        let debut = Instant::now();
        assert!(!meme_selection(None, debut));
        assert!(meme_selection(
            Some(debut),
            debut + Duration::from_millis(30)
        ));
        assert!(!meme_selection(Some(debut), debut + Duration::from_secs(2)));
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

    /// Le morceau s'ouvre en arrière-plan ; un fichier introuvable est sauté
    /// et c'est le suivant qui s'ouvre.
    #[test]
    fn le_morceau_s_ouvre_en_arriere_plan_et_saute_les_illisibles() {
        let ctx = egui::Context::default();
        let _tour = mon_tour();
        let mut app = LecteurApp::new(&ctx);
        let present = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glissando.mp3");
        app.liste
            .ajouter(vec![PathBuf::from("introuvable.mp3"), present]);
        app.jouer(0);
        assert!(app.etat() == Etat::Chargement);
        let debut = std::time::Instant::now();
        while app.chargement.is_some() {
            assert!(debut.elapsed() < Duration::from_secs(5), "jamais ouvert");
            std::thread::sleep(Duration::from_millis(5));
            app.suivre_le_chargement();
        }
        assert_eq!(app.liste.courant(), Some(1));
        if app.sortie.is_some() {
            // Un PC avec du son : le morceau joue, et on dit lequel a été sauté.
            assert!(app.morceau.is_some());
            assert!(app
                .message
                .starts_with("Impossible de lire « introuvable.mp3 »"));
        }
        // Pause pendant l'ouverture : retenue pour le démarrage (et Lecture l'annule).
        app.jouer(1);
        app.pause();
        assert!(app.chargement.as_ref().unwrap().en_pause);
        app.lecture();
        assert!(!app.chargement.as_ref().unwrap().en_pause);
        // Un autre morceau demandé : la lecture du précédent est abandonnée.
        let abandonne = app.chargement.as_ref().unwrap().abandonne.clone();
        app.jouer(1);
        assert!(abandonne.load(Ordering::Relaxed));
        // Stop pendant l'ouverture : rien ne se lance ensuite.
        let abandonne = app.chargement.as_ref().unwrap().abandonne.clone();
        app.stop();
        assert!(abandonne.load(Ordering::Relaxed));
        std::thread::sleep(Duration::from_millis(100));
        app.suivre_le_chargement();
        assert!(app.etat() == Etat::Arrete);
    }

    /// La sortie son meurt (mise en veille, casque débranché) : au bout d'une
    /// seconde, l'appli en ouvre une nouvelle et le morceau reprend au même
    /// endroit, en pause s'il l'était. (Seulement sur un PC qui a du son.)
    #[test]
    fn une_sortie_morte_est_remplacee_et_le_morceau_reprend() {
        let ctx = egui::Context::default();
        let _tour = mon_tour();
        let mut app = LecteurApp::new(&ctx);
        if app.sortie.is_none() {
            return; // pas de son sur ce PC (machine de test)
        }
        let chemin = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glissando.mp3");
        app.morceau = Some(Morceau::charger(&chemin, &AtomicBool::new(false)).unwrap());
        app.freinage = false; // pause immédiate
        app.demarrer(false).unwrap();
        app.pause();
        app.telecommande.as_ref().unwrap().aller_a(1.0);
        let avant = app.position();

        let telecommande = app.telecommande.clone().unwrap();
        // Vrai tant que c'est encore l'ancienne platine qui joue.
        let ancienne_platine = |app: &LecteurApp| {
            telecommande.aller_a(0.5);
            let oui = app.position() == telecommande.position();
            telecommande.aller_a(1.0);
            oui
        };
        assert!(ancienne_platine(&app));

        // La sortie meurt : le compte du témoin ne bouge plus.
        let debut = Instant::now();
        app.prochaine_verification = debut + Duration::from_secs(60);
        app.veilleur = Veilleur::new(debut);
        app.sortie.as_mut().unwrap().figer();
        app.surveiller_sortie(debut);
        app.surveiller_sortie(debut + Duration::from_millis(900));
        assert!(ancienne_platine(&app), "remplacée trop tôt");
        assert_eq!(app.sortie.as_ref().unwrap().battements(), 0);

        // Au bout d'une seconde : une nouvelle sortie qui tourne, avec une
        // nouvelle platine au même endroit, toujours en pause.
        app.surveiller_sortie(debut + sortie::SILENCE_MAX);
        assert!(!ancienne_platine(&app), "pas remplacée");
        std::thread::sleep(Duration::from_millis(300));
        assert!(app.sortie.as_ref().unwrap().battements() > 0);
        assert!(app.etat() == Etat::Pause);
        assert_eq!(app.position(), avant);
    }

    #[test]
    fn une_option_est_activee_sauf_si_on_l_a_desactivee() {
        assert!(option_depuis(None));
        assert!(option_depuis(Some("oui\n")));
        assert!(!option_depuis(Some("non\n")));
    }

    #[test]
    fn une_option_desactivee_par_defaut_attend_qu_on_la_coche() {
        assert!(!option_cochee_depuis(None));
        assert!(option_cochee_depuis(Some("oui\n")));
        assert!(!option_cochee_depuis(Some("non\n")));
    }

    /// Stop avec le freinage : le morceau est tout de suite « Arrêté », le
    /// son ralentit pendant environ 1 s puis tout s'arrête, sans passer au
    /// morceau suivant. Sans freinage, en pause ou au second Stop : on coupe
    /// net. (Seulement sur un PC qui a du son.)
    #[test]
    fn le_stop_freine_puis_s_arrete_sans_passer_au_suivant() {
        let ctx = egui::Context::default();
        let _tour = mon_tour();
        let mut app = LecteurApp::new(&ctx);
        if app.sortie.is_none() {
            return; // pas de son sur ce PC (machine de test)
        }
        let chemin = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glissando.mp3");
        app.liste.ajouter(vec![chemin.clone(), chemin.clone()]);
        app.liste.choisir(0);
        app.morceau = Some(Morceau::charger(&chemin, &AtomicBool::new(false)).unwrap());
        app.freinage = true;

        // Le morceau freine : arrêté pour l'interface, mais le son continue.
        app.demarrer(false).unwrap();
        let telecommande = app.telecommande.clone().unwrap();
        telecommande.aller_a(1.0);
        app.arreter();
        assert!(app.etat() == Etat::Arrete);
        assert!(app.platine_active().is_some_and(|t| t.freine()));
        // Puis il s'arrête pour de bon, et on reste sur le même morceau.
        let debut = Instant::now();
        while app.sink.is_some() {
            assert!(
                debut.elapsed() < Duration::from_secs(3),
                "ne s'arrête jamais"
            );
            std::thread::sleep(Duration::from_millis(10));
            app.suivre_la_fin();
        }
        assert!(app.telecommande.is_none());
        assert!(app.chargement.is_none());
        assert_eq!(app.liste.courant(), Some(0));
        assert!(app.etat() == Etat::Arrete);
        // Le disque a tourné d'environ une demi-seconde de musique en
        // freinant (et pas jusqu'à la fin du morceau). La durée exacte du
        // freinage est vérifiée dans les tests de la platine.
        let position = telecommande.position().as_secs_f64();
        assert!((1.45..1.7).contains(&position), "position {position}");

        // Lecture pendant le freinage : le morceau repart du début, normalement.
        app.demarrer(false).unwrap();
        app.telecommande.as_ref().unwrap().aller_a(2.0);
        app.arreter();
        app.lecture();
        let t = app.telecommande.as_ref().unwrap();
        assert!(!t.freine());
        assert!(t.position() < Duration::from_millis(500));
        assert!(app.etat() == Etat::Lecture);

        // Second Stop pendant le freinage : on coupe net.
        app.arreter();
        app.arreter();
        assert!(app.sink.is_none());

        // En pause : on coupe net.
        app.demarrer(false).unwrap();
        app.pause();
        app.arreter();
        assert!(app.sink.is_none());
        // Sans l'effet platine : le volume baisse en fondu, puis arrêt.
        app.freinage = false;
        app.demarrer(false).unwrap();
        app.arreter();
        let t = app.telecommande.clone().unwrap();
        assert_eq!(t.effet_en_cours(), Some(Effet::Fondu));
        assert!(t.freine());
        assert!(app.etat() == Etat::Arrete);
    }

    /// Pause avec l'effet platine : « En pause » tout de suite, le son
    /// ralentit puis se fige au même endroit. Lecture : le disque repart en
    /// accélérant, depuis la pause comme depuis l'arrêt. Sans l'effet, tout
    /// est immédiat. (Seulement sur un PC qui a du son.)
    #[test]
    fn la_pause_freine_et_la_lecture_accelere() {
        let ctx = egui::Context::default();
        let _tour = mon_tour();
        let mut app = LecteurApp::new(&ctx);
        if app.sortie.is_none() {
            return; // pas de son sur ce PC (machine de test)
        }
        let chemin = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glissando.mp3");
        app.liste.ajouter(vec![chemin.clone()]);
        app.liste.choisir(0);
        app.morceau = Some(Morceau::charger(&chemin, &AtomicBool::new(false)).unwrap());
        app.freinage = true;

        // Pause : le disque ralentit, puis s'arrête au même endroit, en pause.
        app.demarrer(false).unwrap();
        let t = app.telecommande.clone().unwrap();
        t.aller_a(1.0);
        app.pause();
        assert!(app.etat() == Etat::Pause);
        let debut = Instant::now();
        while t.tourne() {
            assert!(
                debut.elapsed() < Duration::from_secs(3),
                "ne se fige jamais"
            );
            std::thread::sleep(Duration::from_millis(10));
            app.suivre_la_fin();
        }
        let position = t.position().as_secs_f64();
        assert!((1.45..1.7).contains(&position), "position {position}");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(t.position().as_secs_f64(), position);
        assert!(app.etat() == Etat::Pause);
        assert!(app.sink.is_some());

        // Lecture : le disque repart en accélérant, du même endroit.
        app.lecture();
        assert!(app.etat() == Etat::Lecture);
        assert!(t.accelere());
        // Pause pendant un Stop qui freine : le Stop va jusqu'au bout.
        app.arreter();
        assert!(t.freine());
        app.pause();
        assert!(t.freine());
        // Lecture après l'arrêt : le morceau repart du début en accélérant.
        app.stop();
        app.lecture();
        let t = app.telecommande.clone().unwrap();
        assert!(t.accelere());
        assert!(t.position() < Duration::from_millis(500));
        // Lecture sans morceau ouvert : il démarrera en accélérant.
        app.oublier_morceau();
        app.lecture();
        assert!(app.chargement.as_ref().unwrap().en_douceur);
        app.stop();
        app.morceau = Some(Morceau::charger(&chemin, &AtomicBool::new(false)).unwrap());

        // Sans l'effet platine : pause et lecture en fondu du volume.
        app.freinage = false;
        app.demarrer(false).unwrap();
        let t = app.telecommande.clone().unwrap();
        app.pause();
        assert!(t.en_pause() && t.tourne());
        assert_eq!(t.effet_en_cours(), Some(Effet::Fondu));
        app.lecture();
        assert!(!t.en_pause() && t.accelere());
        assert_eq!(t.effet_en_cours(), Some(Effet::Fondu));
        app.stop();
        app.lecture();
        let t = app.telecommande.as_ref().unwrap();
        assert_eq!(t.effet_en_cours(), Some(Effet::Fondu));
    }

    /// Son du scratch coupé : pendant le glissement de la barre, la musique
    /// continue normalement ; activé, elle suit la souris.
    #[test]
    fn le_son_du_scratch_peut_etre_coupe() {
        let ctx = egui::Context::default();
        let _tour = mon_tour();
        let mut app = LecteurApp::new(&ctx);
        let morceau = Arc::new(Morceau::brut(
            (0..10_000).map(|i| i as i16).collect(),
            1,
            1000,
        ));
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau);
        telecommande.aller_a(5.0);

        app.scratch = false;
        app.glisser(&telecommande, 2.0);
        for _ in 0..500 {
            platine.next();
        }
        assert_eq!(app.position_glissee, Some(2.0));
        assert_eq!(telecommande.position(), Duration::from_millis(5500));

        app.scratch = true;
        app.glisser(&telecommande, 2.0);
        for _ in 0..1000 {
            platine.next();
        }
        let position = telecommande.position().as_secs_f64();
        assert!((position - 2.0).abs() < 0.01, "position {position}");
    }

    #[test]
    fn le_fondu_dure_3_secondes_sauf_pour_les_morceaux_courts() {
        let s = Duration::from_secs;
        assert_eq!(duree_de_l_enchainement(s(200), s(180)), s(3));
        assert_eq!(duree_de_l_enchainement(s(4), s(180)), s(2));
        assert_eq!(duree_de_l_enchainement(s(200), s(2)), s(1));
    }

    /// Volume égalisé et fondu enchaîné : pendant le fondu, chaque morceau
    /// garde son propre volume égalisé (celui qui s'éteint n'est pas
    /// réglé au volume du nouveau). (Seulement sur un PC qui a du son.)
    #[test]
    fn le_volume_egalise_suit_chaque_morceau_pendant_le_fondu() {
        let ctx = egui::Context::default();
        let _tour = mon_tour();
        let mut app = LecteurApp::new(&ctx);
        if app.sortie.is_none() {
            return; // pas de son sur ce PC (machine de test)
        }
        let fort = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glissando.mp3");
        let dossier = std::env::temp_dir().join("lecteur_mp3_test_egalisation_fondu");
        std::fs::create_dir_all(&dossier).unwrap();
        let calme = dossier.join("calme.mp3");
        std::fs::copy(&fort, &calme).unwrap();
        let mesure = |gain_db| egalisation::Mesure {
            gain_db,
            crete: 0.1,
        };
        app.volumes.fixer(&fort, mesure(-6.0));
        app.volumes.fixer(&calme, mesure(3.0));
        let (gain_fort, gain_calme) = (mesure(-6.0).gain(), mesure(3.0).gain());

        app.liste.ajouter(vec![fort.clone(), calme.clone()]);
        app.liste.choisir(0);
        app.morceau = Some(Morceau::charger(&fort, &AtomicBool::new(false)).unwrap());
        app.egalisation = true;
        app.enchainement = true;
        app.demarrer(false).unwrap();
        let premier = app.telecommande.clone().unwrap();
        assert_eq!(premier.egalisation(), Some(gain_fort));

        let debut = Instant::now();
        while app.sortant.is_none() {
            assert!(debut.elapsed() < Duration::from_secs(5), "pas de fondu");
            std::thread::sleep(Duration::from_millis(5));
            app.preparer_l_enchainement();
            app.suivre_l_egalisation();
            app.suivre_la_fin();
        }
        app.suivre_l_egalisation();
        let second = app.telecommande.clone().unwrap();
        assert_eq!(second.egalisation(), Some(gain_calme));
        assert_eq!(premier.egalisation(), Some(gain_fort));

        // Option décochée : le morceau en cours revient au volume d'origine.
        app.egalisation = false;
        app.suivre_l_egalisation();
        assert_eq!(second.egalisation(), Some(1.0));
        app.stop();
    }

    /// Avec le fondu enchaîné, un peu avant la fin, le morceau suivant est
    /// lancé pendant que l'autre s'éteint ; la liste passe tout de suite au
    /// suivant. ⏭ pendant le fondu coupe net. (Seulement sur un PC qui a du son.)
    #[test]
    fn le_fondu_enchaine_lance_le_suivant_avant_la_fin() {
        let ctx = egui::Context::default();
        let _tour = mon_tour();
        let mut app = LecteurApp::new(&ctx);
        if app.sortie.is_none() {
            return; // pas de son sur ce PC (machine de test)
        }
        let chemin = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glissando.mp3");
        app.liste
            .ajouter(vec![chemin.clone(), chemin.clone(), chemin.clone()]);
        app.liste.choisir(0);
        app.morceau = Some(Morceau::charger(&chemin, &AtomicBool::new(false)).unwrap());
        app.enchainement = true;
        app.demarrer(false).unwrap();
        let premier = app.telecommande.clone().unwrap();

        // On attend que le suivant soit ouvert et que le fondu commence.
        let debut = Instant::now();
        while app.sortant.is_none() {
            assert!(debut.elapsed() < Duration::from_secs(5), "pas de fondu");
            std::thread::sleep(Duration::from_millis(5));
            app.preparer_l_enchainement();
            app.suivre_la_fin();
        }
        assert!(premier.s_eteint());
        assert_eq!(app.liste.courant(), Some(1));
        assert!(app.etat() == Etat::Lecture);
        // Le premier n'était pas fini : le fondu a commencé avant la fin.
        let duree = app.morceau.as_ref().unwrap().duree();
        assert!(premier.position() < duree);
        let second = app.telecommande.clone().unwrap();
        assert!(second.position() < duree / 2);

        // Pause pendant le fondu : les deux morceaux s'arrêtent, et repartent.
        app.pause();
        assert!(premier.en_pause() && second.en_pause());
        app.lecture();
        assert!(!premier.en_pause() && !second.en_pause());

        // Le premier finit de s'éteindre, tout seul. (La sortie son des
        // tests joue plus vite que la musique : le second peut déjà
        // s'enchaîner au troisième entre-temps.)
        while app.sortant.is_some() && app.liste.courant() == Some(1) {
            assert!(debut.elapsed() < Duration::from_secs(10), "ne s'éteint pas");
            std::thread::sleep(Duration::from_millis(5));
            app.preparer_l_enchainement();
            app.suivre_la_fin();
        }

        // ⏭ pendant un fondu coupe tout de suite le morceau qui s'éteint.
        app.stop();
        app.liste.choisir(0);
        app.demarrer(false).unwrap();
        while app.sortant.is_none() {
            assert!(debut.elapsed() < Duration::from_secs(15), "pas de fondu");
            std::thread::sleep(Duration::from_millis(5));
            app.preparer_l_enchainement();
            app.suivre_la_fin();
        }
        assert_eq!(app.liste.courant(), Some(1));
        app.suivant();
        assert!(app.sortant.is_none());
        assert!(app.platine_active().is_none()); // le suivant s'ouvre

        // Sans fondu enchaîné, rien n'est lancé avant la fin.
        app.stop();
        app.liste.choisir(0);
        app.enchainement = false;
        app.morceau = Some(Morceau::charger(&chemin, &AtomicBool::new(false)).unwrap());
        app.demarrer(false).unwrap();
        app.telecommande.as_ref().unwrap().aller_a(2.5);
        for _ in 0..50 {
            app.preparer_l_enchainement();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(app.sortant.is_none() && app.a_suivre.is_none());
        assert_eq!(app.liste.courant(), Some(0));
    }
}
