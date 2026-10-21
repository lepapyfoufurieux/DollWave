//! La bibliothèque : tous les MP3 du dossier de musique, rangés par
//! sous-dossier, par artiste ou par album, qu'on ajoute à la liste de
//! lecture d'un clic.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;

use crate::liste::{nom_du_fichier, rangee};
use crate::playlists::{menu_clic_droit, ChoixDuMenu, MorceauxGlisses};
use crate::regroupement::{regrouper, Album, Regroupement};
use crate::systeme::{dossier_musique, fichier_de_reglage};
use crate::tags::TagsConnus;

/// Au-delà de ce nombre de résultats de recherche, on n'affiche que les
/// premiers (sinon la fenêtre ralentirait) ; « Ajouter » les prend tous.
const RESULTATS_MAX: usize = 500;

/// Un dossier de la bibliothèque, avec ce qu'il contient.
#[derive(Default)]
struct Dossier {
    chemin: PathBuf,
    nom: String,
    /// Les sous-dossiers qui contiennent des MP3 (les autres sont ignorés).
    sous_dossiers: Vec<Dossier>,
    /// Les MP3 rangés directement dans ce dossier.
    morceaux: Vec<PathBuf>,
    /// Le nombre de morceaux, sous-dossiers compris.
    total: usize,
}

impl Dossier {
    /// Parcourt un dossier et ses sous-dossiers. Les dossiers illisibles
    /// sont simplement ignorés. Tout est trié par nom, sans tenir compte
    /// des majuscules.
    fn lire(chemin: &Path) -> Dossier {
        let mut dossier = Dossier {
            chemin: chemin.to_path_buf(),
            nom: nom_du_fichier(chemin),
            ..Default::default()
        };
        if let Ok(contenu) = std::fs::read_dir(chemin) {
            for entree in contenu.flatten() {
                let Ok(genre) = entree.file_type() else {
                    continue;
                };
                let chemin = entree.path();
                // `file_type` ne suit pas les raccourcis vers d'autres dossiers :
                // un raccourci qui ramènerait vers un dossier parent ne peut pas
                // nous faire tourner en rond.
                if genre.is_dir() {
                    let sous_dossier = Dossier::lire(&chemin);
                    if sous_dossier.total > 0 {
                        dossier.sous_dossiers.push(sous_dossier);
                    }
                } else if est_un_mp3(&chemin) {
                    dossier.morceaux.push(chemin);
                }
            }
        }
        // « cached » : chaque nom n'est mis en minuscules qu'une fois.
        dossier
            .sous_dossiers
            .sort_by_cached_key(|d| d.nom.to_lowercase());
        dossier
            .morceaux
            .sort_by_cached_key(|m| nom_du_fichier(m).to_lowercase());
        dossier.total =
            dossier.morceaux.len() + dossier.sous_dossiers.iter().map(|d| d.total).sum::<usize>();
        dossier
    }

    /// Tous les morceaux du dossier, dans l'ordre où ils sont affichés :
    /// d'abord ceux des sous-dossiers, puis les siens.
    fn tous_les_morceaux(&self, resultat: &mut Vec<PathBuf>) {
        for sous_dossier in &self.sous_dossiers {
            sous_dossier.tous_les_morceaux(resultat);
        }
        resultat.extend(self.morceaux.iter().cloned());
    }

    /// Comme `tous_les_morceaux`, dans une nouvelle liste.
    fn liste(&self) -> Vec<PathBuf> {
        let mut morceaux = Vec::new();
        self.tous_les_morceaux(&mut morceaux);
        morceaux
    }
}

/// Tous les MP3 d'un dossier et de ses sous-dossiers, dans l'ordre où la
/// bibliothèque les affiche (et les ajoute avec « ➕ tout »).
pub fn mp3_du_dossier(chemin: &Path) -> Vec<PathBuf> {
    let mut morceaux = Vec::new();
    Dossier::lire(chemin).tous_les_morceaux(&mut morceaux);
    morceaux
}

/// Un morceau tel qu'on le cherche : son chemin, son dossier (par rapport
/// au dossier de musique) et le texte dans lequel on cherche, en minuscules.
struct Entree {
    chemin: PathBuf,
    dossier: String,
    texte: String,
}

/// Ce qu'on a trouvé en lisant le dossier de musique.
struct Contenu {
    /// Le dossier existe-t-il ?
    existe: bool,
    racine: Dossier,
    /// Tous les morceaux à la suite, dans l'ordre de l'arbre, pour la recherche.
    tous: Vec<Entree>,
}

impl Contenu {
    fn lire(dossier: &Path) -> Contenu {
        let racine = Dossier::lire(dossier);
        let mut chemins = Vec::new();
        racine.tous_les_morceaux(&mut chemins);
        let tous = chemins
            .into_iter()
            .map(|chemin| {
                let dossier = chemin
                    .parent()
                    .and_then(|p| p.strip_prefix(&racine.chemin).ok())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                // On cherche dans le nom sans « .mp3 », sinon taper « mp3 »
                // (ou « p3 ») trouverait tous les morceaux.
                let nom = chemin.file_stem().unwrap_or_default().to_string_lossy();
                let texte = format!("{dossier} {nom}").to_lowercase();
                Entree {
                    chemin,
                    dossier,
                    texte,
                }
            })
            .collect();
        Contenu {
            existe: dossier.is_dir(),
            racine,
            tous,
        }
    }
}

/// Les numéros des morceaux qui contiennent tous les mots recherchés,
/// dans leur nom, le nom de leur dossier ou leurs tags (titre, artiste,
/// album), sans tenir compte des majuscules. Par exemple « daft discovery »
/// trouve « Daft Punk\Discovery\One More Time.mp3 ».
/// `tags` donne le texte des tags d'un morceau, en minuscules.
fn chercher<'t>(tous: &[Entree], recherche: &str, tags: impl Fn(&Path) -> &'t str) -> Vec<usize> {
    let mots: Vec<String> = recherche
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    tous.iter()
        .enumerate()
        .filter(|(_, entree)| {
            // On ne regarde les tags que pour les mots absents du nom.
            let mut manquants = mots
                .iter()
                .filter(|mot| !entree.texte.contains(mot.as_str()))
                .peekable();
            if manquants.peek().is_none() {
                return true;
            }
            let tags = tags(&entree.chemin);
            manquants.all(|mot| tags.contains(mot.as_str()))
        })
        .map(|(n, _)| n)
        .collect()
}

/// Comment la bibliothèque est rangée.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Vue {
    Dossiers,
    Artistes,
    Albums,
}

impl Vue {
    const TOUTES: [Vue; 3] = [Vue::Dossiers, Vue::Artistes, Vue::Albums];

    fn libelle(self) -> &'static str {
        match self {
            Vue::Dossiers => "📁 Dossiers",
            Vue::Artistes => "🎤 Artistes",
            Vue::Albums => "💿 Albums",
        }
    }

    fn aide(self) -> &'static str {
        match self {
            Vue::Dossiers => "Ranger la musique par dossier",
            Vue::Artistes => "Ranger la musique par artiste, puis par album (d'après les tags)",
            Vue::Albums => "Ranger la musique par album (d'après les tags)",
        }
    }

    /// Le mot écrit dans le fichier de réglage.
    fn code(self) -> &'static str {
        match self {
            Vue::Dossiers => "dossiers",
            Vue::Artistes => "artistes",
            Vue::Albums => "albums",
        }
    }

    /// La vue mémorisée ; par dossier si le fichier est absent ou abîmé.
    fn depuis_le_code(code: &str) -> Vue {
        Vue::TOUTES
            .into_iter()
            .find(|v| v.code() == code.trim())
            .unwrap_or(Vue::Dossiers)
    }
}

pub struct Bibliotheque {
    /// Le dossier de musique.
    dossier: PathBuf,
    /// Ce qu'on y a trouvé (rien tant que la lecture n'est pas finie).
    contenu: Option<Contenu>,
    /// Pendant que le dossier est lu en arrière-plan : de quoi recevoir le
    /// résultat quand ce sera fini.
    en_cours: Option<Receiver<Contenu>>,
    /// Le texte tapé dans le champ de recherche.
    recherche: String,
    /// Les résultats de la recherche, et la recherche à laquelle ils
    /// correspondent (avec la version des tags connus à ce moment-là) :
    /// on ne recalcule que si l'une ou l'autre a changé.
    resultats: Vec<usize>,
    recherche_faite: Option<(String, u64)>,
    /// Les dossiers et morceaux mis en favori (★) : ils remontent en tête
    /// de leur dossier.
    favoris: HashSet<PathBuf>,
    /// La hauteur d'une ligne de morceau à l'écran, pour réserver la place
    /// de celles qu'on ne dessine pas (hors de l'écran).
    hauteur_ligne: Option<f32>,
    /// Pareil pour une ligne de dossier, d'artiste ou d'album repliée.
    hauteur_entete: Option<f32>,
    /// Rangée par dossier, par artiste ou par album.
    vue: Vue,
    /// La musique rangée par artiste et par album, et la version des tags
    /// connus à ce moment-là : on ne range à nouveau que si de nouveaux
    /// tags sont arrivés.
    regroupement: Option<(Regroupement, u64)>,
}

impl Bibliotheque {
    /// Reprend le dossier mémorisé la dernière fois, ou à défaut le dossier
    /// Musique du système, et commence à le lire.
    pub fn new(ctx: &egui::Context) -> Self {
        let dossier = fichier_de_reglage(REGLAGE_DOSSIER)
            .and_then(|f| lire_reglage(&f))
            .unwrap_or_else(dossier_musique);
        let favoris = fichier_de_reglage(REGLAGE_FAVORIS)
            .map(|f| lire_favoris(&f))
            .unwrap_or_default();
        let vue = fichier_de_reglage(REGLAGE_VUE)
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map_or(Vue::Dossiers, |code| Vue::depuis_le_code(&code));
        let mut bibliotheque = Self {
            dossier,
            favoris,
            contenu: None,
            en_cours: None,
            recherche: String::new(),
            resultats: Vec::new(),
            recherche_faite: None,
            hauteur_ligne: None,
            hauteur_entete: None,
            vue,
            regroupement: None,
        };
        bibliotheque.actualiser(ctx);
        bibliotheque
    }

    /// Relit le dossier de musique. Ça se fait en arrière-plan, pour que la
    /// fenêtre ne se fige pas s'il y a beaucoup de musique.
    fn actualiser(&mut self, ctx: &egui::Context) {
        let (envoi, reception) = mpsc::channel();
        let dossier = self.dossier.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // Si entre-temps on a relu ou changé de dossier, plus personne
            // n'attend ce résultat : l'envoi échoue sans conséquence.
            let _ = envoi.send(Contenu::lire(&dossier));
            ctx.request_repaint(); // pour afficher le résultat tout de suite
        });
        self.en_cours = Some(reception);
    }

    /// Ouvre la fenêtre Windows de choix de dossier, mémorise le dossier
    /// choisi et le lit.
    fn changer_de_dossier(&mut self, ctx: &egui::Context) {
        let Some(dossier) = rfd::FileDialog::new()
            .set_directory(&self.dossier)
            .pick_folder()
        else {
            return; // l'utilisateur a annulé
        };
        memoriser(REGLAGE_DOSSIER, &dossier.to_string_lossy());
        self.dossier = dossier;
        self.contenu = None;
        self.recherche.clear();
        self.actualiser(ctx);
    }

    /// Dessine la bibliothèque. Renvoie ce qu'on y a demandé en cliquant.
    /// `dans_la_liste` : les morceaux déjà dans la liste, marqués d'un ✔.
    /// `tags` : les tags des morceaux, pour les nommer et pour la recherche.
    /// `playlists` : les noms des playlists, proposés au clic droit.
    pub fn afficher(
        &mut self,
        ui: &mut egui::Ui,
        dans_la_liste: &HashSet<&Path>,
        tags: &TagsConnus,
        playlists: &[String],
    ) -> Demandes {
        dater_le_dernier_clic(ui.ctx());

        // La lecture en arrière-plan est-elle finie ?
        if let Some(reception) = &self.en_cours {
            match reception.try_recv() {
                Ok(contenu) => {
                    // On lit les tags de toute la bibliothèque en arrière-plan,
                    // pour que la recherche les trouve.
                    tags.demander(contenu.tous.iter().map(|e| e.chemin.as_path()));
                    self.contenu = Some(contenu);
                    self.en_cours = None;
                    self.recherche_faite = None; // la recherche est à refaire
                    self.regroupement = None; // et le rangement aussi
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.en_cours = None,
            }
        }

        ui.horizontal(|ui| {
            ui.strong("Bibliothèque");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("📁 Changer de dossier…")
                    .on_hover_text("Choisir le dossier où est rangée ta musique")
                    .clicked()
                {
                    self.changer_de_dossier(ui.ctx());
                }
                if ui
                    .button("🔄")
                    .on_hover_text("Relire le dossier (après avoir ajouté de la musique)")
                    .clicked()
                {
                    self.actualiser(ui.ctx());
                }
            });
        });
        ui.label(
            egui::RichText::new(self.dossier.display().to_string())
                .small()
                .weak(),
        );

        // De droite à gauche : d'abord le bouton ✖, puis le champ prend
        // exactement la place qui reste (sans quoi le panneau s'élargirait
        // tout seul à chaque image).
        let ligne = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        let de_droite_a_gauche = egui::Layout::right_to_left(egui::Align::Center);
        ui.allocate_ui_with_layout(ligne, de_droite_a_gauche, |ui| {
            if !self.recherche.is_empty()
                && ui
                    .small_button("✖")
                    .on_hover_text("Effacer la recherche")
                    .clicked()
            {
                self.recherche.clear();
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.recherche)
                    // Un nom fixe, pour que le champ garde la main quand le
                    // bouton ✖ apparaît à côté.
                    .id(egui::Id::new("recherche_bibliotheque"))
                    .hint_text("🔍 Rechercher un morceau, un dossier…")
                    .desired_width(ui.available_width()),
            );
        });
        ui.horizontal(|ui| {
            for vue in Vue::TOUTES {
                if ui
                    .selectable_label(self.vue == vue, vue.libelle())
                    .on_hover_text(vue.aide())
                    .clicked()
                    && self.vue != vue
                {
                    self.vue = vue;
                    memoriser(REGLAGE_VUE, vue.code());
                }
            }
        });
        ui.separator();

        // On met les favoris de côté le temps de dessiner les lignes, qui
        // en ont besoin pour afficher les étoiles.
        let favoris = std::mem::take(&mut self.favoris);
        let mut lignes = Lignes {
            dans_la_liste,
            tags,
            favoris: &favoris,
            playlists,
            a_ajouter: Vec::new(),
            etoile: None,
            vers_playlist: None,
            tags_a_modifier: None,
            infos_a_voir: None,
            hauteur: self.hauteur_ligne,
            hauteur_entete: self.hauteur_entete,
        };
        self.afficher_contenu(ui, &mut lignes);
        let Lignes {
            a_ajouter,
            etoile,
            vers_playlist,
            tags_a_modifier,
            infos_a_voir,
            hauteur,
            hauteur_entete,
            ..
        } = lignes;
        self.hauteur_ligne = hauteur;
        self.hauteur_entete = hauteur_entete;
        self.favoris = favoris;

        // Un clic sur une étoile : on ajoute ou retire le favori, et on le
        // mémorise tout de suite.
        if let Some(chemin) = etoile {
            if !self.favoris.remove(&chemin) {
                self.favoris.insert(chemin);
            }
            memoriser(REGLAGE_FAVORIS, &texte_des_favoris(&self.favoris));
        }
        Demandes {
            a_ajouter,
            vers_playlist,
            tags_a_modifier,
            infos_a_voir,
        }
    }

    /// Dessine les morceaux : l'arbre des dossiers, les artistes, les
    /// albums, ou les résultats de la recherche.
    fn afficher_contenu(&mut self, ui: &mut egui::Ui, lignes: &mut Lignes) {
        if self.en_cours.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Lecture du dossier de musique…");
            });
            return;
        }
        let Some(contenu) = &self.contenu else {
            return;
        };
        if !contenu.existe {
            ui.label("Ce dossier est introuvable. Choisis ton dossier de musique avec « 📁 Changer de dossier… ».");
            return;
        }
        if contenu.tous.is_empty() {
            ui.label("Aucun fichier MP3 dans ce dossier.");
            return;
        }

        let recherche = self.recherche.trim();
        if recherche.is_empty() && self.vue != Vue::Dossiers {
            // On range à nouveau quand de nouveaux tags sont arrivés.
            let version = lignes.tags.version();
            if self.regroupement.as_ref().map(|(_, v)| *v) != Some(version) {
                let textes = lignes.tags.pour_chercher();
                let regroupement =
                    regrouper(contenu.tous.iter().map(|e| &e.chemin), |c| textes.tags(c));
                self.regroupement = Some((regroupement, version));
            }
            let Some((regroupement, _)) = &self.regroupement else {
                return;
            };
            afficher_regroupement(ui, self.vue, regroupement, contenu.tous.len(), lignes);
            return;
        }
        if recherche.is_empty() {
            ui.horizontal(|ui| {
                ui.label(format!("{} morceaux", contenu.tous.len()));
                let bouton = ui
                    .small_button("➕ Tout ajouter")
                    .on_hover_text("Ajouter toute la bibliothèque à la liste de lecture");
                if clic_simple(ui, &bouton) {
                    contenu.racine.tous_les_morceaux(&mut lignes.a_ajouter);
                }
            });
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    afficher_dossier(ui, &contenu.racine, lignes);
                });
            return;
        }

        // Pendant que les tags arrivent, la recherche se refait de temps en temps.
        let faite = (recherche.to_string(), lignes.tags.version());
        if self.recherche_faite.as_ref() != Some(&faite) {
            let textes = lignes.tags.pour_chercher();
            self.resultats = chercher(&contenu.tous, recherche, |m| textes.de(m));
            self.recherche_faite = Some(faite);
        }
        if self.resultats.is_empty() {
            ui.label("Aucun morceau ne correspond à cette recherche.");
            return;
        }
        ui.horizontal(|ui| {
            let nombre = self.resultats.len();
            ui.label(if nombre == 1 {
                "1 résultat".to_string()
            } else {
                format!("{nombre} résultats")
            });
            let bouton = ui
                .small_button("➕ Tout ajouter")
                .on_hover_text("Ajouter tous les résultats à la liste de lecture");
            if clic_simple(ui, &bouton) {
                // Dans l'ordre de la bibliothèque, sans faire passer les
                // favoris devant.
                lignes.a_ajouter.extend(
                    self.resultats
                        .iter()
                        .map(|&n| contenu.tous[n].chemin.clone()),
                );
            }
        });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let tous = &contenu.tous;
                let resultats =
                    favoris_d_abord(&self.resultats, |&n| &tous[n].chemin, lignes.favoris);
                let mut hauteur = lignes.hauteur;
                for &n in resultats.into_iter().take(RESULTATS_MAX) {
                    let entree = &tous[n];
                    rangee(ui, &mut hauteur, n, |ui| {
                        ligne_morceau(ui, &entree.chemin, lignes);
                        ui.label(egui::RichText::new(&entree.dossier).small().weak());
                    });
                }
                lignes.hauteur = hauteur;
                if self.resultats.len() > RESULTATS_MAX {
                    ui.label(format!(
                        "… et {} autres : précise ta recherche pour les voir.",
                        self.resultats.len() - RESULTATS_MAX
                    ));
                }
            });
    }
}

/// Ce qu'on a demandé en cliquant dans la bibliothèque.
pub struct Demandes {
    /// Les morceaux à ajouter à la liste de lecture (vide si aucun).
    pub a_ajouter: Vec<PathBuf>,
    /// Des morceaux à ajouter à une playlist (clic droit), et son nom.
    pub vers_playlist: Option<(String, Vec<PathBuf>)>,
    /// Les morceaux dont on veut modifier les tags (clic droit).
    pub tags_a_modifier: Option<Vec<PathBuf>>,
    /// Le morceau dont on veut voir les infos du fichier (clic droit).
    pub infos_a_voir: Option<PathBuf>,
}

/// Ce dont les lignes ont besoin pour se dessiner, et ce qu'on y a cliqué.
struct Lignes<'a> {
    /// Les morceaux déjà dans la liste de lecture, marqués d'un ✔.
    dans_la_liste: &'a HashSet<&'a Path>,
    tags: &'a TagsConnus,
    favoris: &'a HashSet<PathBuf>,
    /// Les noms des playlists, proposés au clic droit.
    playlists: &'a [String],
    /// Les morceaux à ajouter à la liste de lecture.
    a_ajouter: Vec<PathBuf>,
    /// Le dossier ou le morceau dont on a cliqué l'étoile.
    etoile: Option<PathBuf>,
    /// Des morceaux à ajouter à une playlist (clic droit), et son nom.
    vers_playlist: Option<(String, Vec<PathBuf>)>,
    /// Les morceaux dont on veut modifier les tags (clic droit).
    tags_a_modifier: Option<Vec<PathBuf>>,
    /// Le morceau dont on veut voir les infos du fichier (clic droit).
    infos_a_voir: Option<PathBuf>,
    /// La hauteur d'une ligne de morceau (voir `liste::rangee`).
    hauteur: Option<f32>,
    /// La hauteur d'une ligne de dossier, d'artiste ou d'album repliée.
    hauteur_entete: Option<f32>,
}

/// Quand on commence à faire glisser un dossier ou un morceau, on emporte
/// ses morceaux, pour les lâcher sur une playlist.
fn glisser(reponse: &egui::Response, morceaux: impl FnOnce() -> Vec<PathBuf>) {
    if reponse.drag_started() {
        egui::DragAndDrop::set_payload(&reponse.ctx, MorceauxGlisses(morceaux()));
    }
}

/// Le menu du clic droit sur un dossier, un artiste, un album ou un
/// morceau : « Infos du fichier… » (pour un morceau seul, `infos`),
/// « Modifier les tags… », puis « Ajouter à la playlist » et le nom de chaque
/// playlist. `morceaux` donne les morceaux concernés (on ne le calcule que si
/// on a choisi quelque chose).
fn menu(
    reponse: &egui::Response,
    lignes: &mut Lignes,
    modifier: &str,
    infos: bool,
    morceaux: impl FnOnce() -> Vec<PathBuf>,
) {
    match menu_clic_droit(reponse, lignes.playlists, modifier, infos) {
        Some(ChoixDuMenu::Playlist(nom)) => lignes.vers_playlist = Some((nom, morceaux())),
        Some(ChoixDuMenu::ModifierLesTags) => lignes.tags_a_modifier = Some(morceaux()),
        Some(ChoixDuMenu::Infos) => lignes.infos_a_voir = morceaux().into_iter().next(),
        None => {}
    }
}

/// Les éléments mis en favori d'abord, puis les autres. Chaque groupe
/// garde l'ordre de départ (alphabétique).
fn favoris_d_abord<'a, T>(
    elements: &'a [T],
    chemin: impl Fn(&'a T) -> &'a Path,
    favoris: &HashSet<PathBuf>,
) -> Vec<&'a T> {
    let (mut tries, autres): (Vec<&T>, Vec<&T>) =
        elements.iter().partition(|&e| favoris.contains(chemin(e)));
    tries.extend(autres);
    tries
}

/// Dessine le contenu d'un dossier : ses sous-dossiers, qu'on peut déplier
/// (fermés au départ), puis ses morceaux. Les favoris passent en tête.
fn afficher_dossier(ui: &mut egui::Ui, dossier: &Dossier, lignes: &mut Lignes) {
    for sous_dossier in favoris_d_abord(&dossier.sous_dossiers, |d| &d.chemin, lignes.favoris) {
        let id = ui.make_persistent_id(&sous_dossier.chemin);
        entete_depliable(
            ui,
            lignes,
            id,
            Some(&sous_dossier.chemin),
            format!("📁 {} ({})", sous_dossier.nom, sous_dossier.total),
            "Ajouter tout ce dossier à la liste de lecture",
            || sous_dossier.liste(),
            |ui, lignes| afficher_dossier(ui, sous_dossier, lignes),
        );
    }
    afficher_morceaux(ui, &dossier.morceaux, lignes);
}

/// Des morceaux, un par ligne, les favoris en tête. Seuls ceux à l'écran
/// sont dessinés.
fn afficher_morceaux(ui: &mut egui::Ui, morceaux: &[PathBuf], lignes: &mut Lignes) {
    let mut hauteur = lignes.hauteur;
    for morceau in favoris_d_abord(morceaux, PathBuf::as_path, lignes.favoris) {
        rangee(ui, &mut hauteur, morceau, |ui| {
            ligne_morceau(ui, morceau, lignes)
        });
    }
    lignes.hauteur = hauteur;
}

/// La ligne d'un dossier, d'un artiste ou d'un album, qu'on peut déplier
/// (fermée au départ) : son étoile (s'il peut être mis en favori), son nom
/// (un clic le déplie ; on peut le glisser sur une playlist, ou faire un
/// clic droit), et « ➕ tout ». Une fois déplié, `contenu` dessine ce qu'il
/// contient. `morceaux` donne tous ses morceaux, dans l'ordre affiché.
#[allow(clippy::too_many_arguments)]
fn entete_depliable(
    ui: &mut egui::Ui,
    lignes: &mut Lignes,
    id: egui::Id,
    favori: Option<&Path>,
    nom: String,
    aide_tout: &str,
    morceaux: impl Fn() -> Vec<PathBuf>,
    contenu: impl FnOnce(&mut egui::Ui, &mut Lignes),
) {
    let etat =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    // Replié et hors de l'écran : on réserve juste sa place, sans le dessiner
    // (une grosse bibliothèque peut avoir des milliers d'albums).
    let replie = etat.openness(ui.ctx()) == 0.0;
    if replie {
        if let Some(h) = lignes.hauteur_entete {
            let taille = egui::vec2(ui.available_width(), h);
            if !ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, taille)) {
                ui.allocate_exact_size(taille, egui::Sense::hover());
                return;
            }
        }
    }
    let mut basculer = false;
    let mut entete = etat.show_header(ui, |ui| {
        if let Some(favori) = favori {
            etoile(ui, favori, lignes);
        }
        // Un clic sur le nom le déplie ou le replie, comme la flèche.
        let etiquette = ui.add(
            egui::Label::new(nom)
                .selectable(false)
                .sense(egui::Sense::click_and_drag()),
        );
        if clic_simple(ui, &etiquette) {
            basculer = true;
        }
        glisser(&etiquette, &morceaux);
        menu(
            &etiquette,
            lignes,
            "✏ Modifier les tags de tous ses morceaux…",
            false,
            &morceaux,
        );
        let bouton = ui.small_button("➕ tout").on_hover_text(aide_tout);
        if clic_simple(ui, &bouton) {
            // Dans l'ordre affiché, sans faire passer les favoris devant.
            lignes.a_ajouter.extend(morceaux());
        }
    });
    if basculer {
        entete.toggle();
    }
    let (_, ligne, _) = entete.body(|ui| contenu(ui, lignes));
    if replie {
        lignes.hauteur_entete = Some(ligne.response.rect.height());
    }
}

/// La vue Artistes ou Albums.
fn afficher_regroupement(
    ui: &mut egui::Ui,
    vue: Vue,
    regroupement: &Regroupement,
    nombre: usize,
    lignes: &mut Lignes,
) {
    let a_lire = regroupement.pas_encore_lus.len();
    ui.horizontal(|ui| {
        ui.label(format!("{nombre} morceaux"));
        let bouton = ui
            .small_button("➕ Tout ajouter")
            .on_hover_text("Ajouter toute la bibliothèque à la liste de lecture");
        if clic_simple(ui, &bouton) {
            match vue {
                Vue::Artistes => {
                    for artiste in &regroupement.artistes {
                        lignes.a_ajouter.extend(artiste.morceaux());
                    }
                }
                _ => {
                    for album in &regroupement.albums {
                        lignes.a_ajouter.extend(album.morceaux.iter().cloned());
                    }
                }
            }
            lignes
                .a_ajouter
                .extend(regroupement.pas_encore_lus.iter().cloned());
        }
    });
    if a_lire > 0 {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("Lecture des tags… {} / {nombre}", nombre - a_lire));
        });
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match vue {
            Vue::Artistes => {
                let artistes = favoris_d_abord(&regroupement.artistes, |a| &a.cle, lignes.favoris);
                for artiste in artistes {
                    let id = ui.make_persistent_id(("artiste", &artiste.cle));
                    entete_depliable(
                        ui,
                        lignes,
                        id,
                        artiste.nom.is_some().then_some(artiste.cle.as_path()),
                        format!("🎤 {} ({})", artiste.nom(), artiste.total),
                        "Ajouter tous les morceaux de cet artiste à la liste de lecture",
                        || artiste.morceaux(),
                        |ui, lignes| {
                            afficher_albums(ui, &artiste.albums, false, lignes);
                        },
                    );
                }
            }
            _ => afficher_albums(ui, &regroupement.albums, true, lignes),
        });
}

/// Des albums, chacun dépliable sur ses morceaux. `avec_artiste` : écrire
/// l'artiste à côté du nom (dans la vue Albums ; pas sous un artiste).
fn afficher_albums(ui: &mut egui::Ui, albums: &[Album], avec_artiste: bool, lignes: &mut Lignes) {
    for album in favoris_d_abord(albums, |a| &a.cle, lignes.favoris) {
        let id = ui.make_persistent_id(("album", &album.cle));
        let nom = match (&album.artiste, avec_artiste) {
            (Some(artiste), true) => {
                format!("💿 {} — {artiste} ({})", album.nom(), album.morceaux.len())
            }
            _ => format!("💿 {} ({})", album.nom(), album.morceaux.len()),
        };
        entete_depliable(
            ui,
            lignes,
            id,
            album.nom.is_some().then_some(album.cle.as_path()),
            nom,
            "Ajouter tout cet album à la liste de lecture",
            || album.morceaux.clone(),
            |ui, lignes| afficher_morceaux(ui, &album.morceaux, lignes),
        );
    }
}

/// L'étoile devant un dossier ou un morceau : ★ s'il est en favori, ☆ sinon.
/// Un clic la change.
fn etoile(ui: &mut egui::Ui, chemin: &Path, lignes: &mut Lignes) {
    let favori = lignes.favoris.contains(chemin);
    let (texte, aide) = if favori {
        (
            egui::RichText::new("★").color(crate::theme::JAUNE),
            "Retirer des favoris",
        )
    } else {
        (
            egui::RichText::new("☆").weak(),
            "Mettre en favori (remonte en tête)",
        )
    };
    let bouton = ui
        .add(egui::Button::new(texte).frame(false))
        .on_hover_text(aide);
    if clic_simple(ui, &bouton) {
        lignes.etoile = Some(chemin.to_path_buf());
    }
}

/// Une ligne pour un morceau : son étoile, puis son nom (d'après ses tags) ;
/// un clic sur le nom l'ajoute à la liste de lecture.
fn ligne_morceau(ui: &mut egui::Ui, chemin: &Path, lignes: &mut Lignes) {
    etoile(ui, chemin, lignes);
    let deja = lignes.dans_la_liste.contains(chemin);
    let texte = format!(
        "{} {}",
        if deja { "✔" } else { "➕" },
        lignes.tags.libelle(chemin)
    );
    let aide = if deja {
        "Déjà dans la liste de lecture (cliquer pour l'ajouter encore une fois ; glisser sur une playlist ou clic droit pour l'y ajouter)"
    } else {
        "Cliquer pour ajouter à la liste de lecture (glisser sur une playlist ou clic droit pour l'y ajouter)"
    };
    // L'infobulle n'est préparée que si la souris est dessus.
    let tags = lignes.tags;
    let ligne = ui
        .selectable_label(false, texte)
        .interact(egui::Sense::drag())
        .on_hover_ui(|ui| {
            ui.label(format!("{}\n\n{aide}", tags.infobulle(chemin)));
        });
    if clic_simple(ui, &ligne) {
        lignes.a_ajouter.push(chemin.to_path_buf());
    }
    glisser(&ligne, || vec![chemin.to_path_buf()]);
    menu(&ligne, lignes, "✏ Modifier les tags…", true, || {
        vec![chemin.to_path_buf()]
    });
}

/// Le dernier clic dans la bibliothèque : sur quel élément, et à quel moment.
#[derive(Clone, Copy)]
struct DernierClic {
    id: egui::Id,
    temps: f64,
    /// Le moment doit-il encore être recalé au début de l'image suivante ?
    a_dater: bool,
}

/// Où l'on range le dernier clic dans la mémoire d'egui.
fn memoire_du_clic() -> egui::Id {
    egui::Id::new("dernier_clic_bibliotheque")
}

/// À appeler au début de chaque image. Un clic peut lancer la lecture, et
/// décoder le morceau fige la fenêtre un instant : le 2e clic d'un double-clic
/// n'est alors vu qu'après, et paraît venir bien plus tard qu'en vrai. On
/// compte donc le délai du double-clic à partir de l'image qui suit le clic,
/// c'est-à-dire une fois le travail du clic terminé.
fn dater_le_dernier_clic(ctx: &egui::Context) {
    let maintenant = ctx.input(|i| i.time);
    ctx.data_mut(|d| {
        if let Some(clic) = d.get_temp_mut_or_default::<Option<DernierClic>>(memoire_du_clic()) {
            if clic.a_dater {
                clic.temps = maintenant;
                clic.a_dater = false;
            }
        }
    });
}

/// Un clic qui compte. Pour egui, un double-clic, c'est deux clics : le
/// second ajouterait le morceau une deuxième fois. On ignore donc un clic
/// sur le même élément que le précédent s'il arrive dans le délai d'un
/// double-clic (voir `dater_le_dernier_clic`). Deux clics rapides sur deux
/// morceaux différents comptent bien tous les deux.
pub fn clic_simple(ui: &egui::Ui, reponse: &egui::Response) -> bool {
    if !reponse.clicked() {
        return false;
    }
    let maintenant = ui.input(|i| i.time);
    let delai = ui.ctx().options(|o| o.input_options.max_double_click_delay);
    let precedent = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<Option<DernierClic>>(memoire_du_clic())
            .replace(DernierClic {
                id: reponse.id,
                temps: maintenant,
                a_dater: true,
            })
    });
    let repetition = precedent.is_some_and(|p| p.id == reponse.id && maintenant - p.temps < delai);
    !repetition
}

/// Le fichier se termine-t-il par .mp3 (ou .MP3, .Mp3…) ?
pub fn est_un_mp3(chemin: &Path) -> bool {
    chemin
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
}

/// Les petits fichiers où l'on mémorise nos réglages.
const REGLAGE_DOSSIER: &str = "dossier_musique.txt";
const REGLAGE_FAVORIS: &str = "favoris.txt";
const REGLAGE_VUE: &str = "vue_bibliotheque.txt";

/// Le dossier mémorisé, s'il y en a un.
fn lire_reglage(fichier: &Path) -> Option<PathBuf> {
    let texte = std::fs::read_to_string(fichier).ok()?;
    let dossier = texte.trim();
    (!dossier.is_empty()).then(|| PathBuf::from(dossier))
}

/// Les favoris mémorisés : un chemin par ligne.
fn lire_favoris(fichier: &Path) -> HashSet<PathBuf> {
    std::fs::read_to_string(fichier)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|ligne| !ligne.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Les favoris sous forme de texte, un chemin par ligne, triés pour que le
/// fichier reste lisible.
fn texte_des_favoris(favoris: &HashSet<PathBuf>) -> String {
    let mut lignes: Vec<String> = favoris
        .iter()
        .map(|f| f.to_string_lossy().into_owned())
        .collect();
    lignes.sort();
    lignes.join("\n")
}

/// Écrit un réglage (en créant le dossier des réglages si besoin).
fn ecrire_reglage(fichier: &Path, texte: &str) -> std::io::Result<()> {
    if let Some(parent) = fichier.parent() {
        std::fs::create_dir_all(parent)?;
    }
    ecrire_sans_risque(fichier, texte.as_bytes())
}

/// Écrit un fichier sans risquer de le perdre : on écrit d'abord une copie
/// à côté (« favoris.txt.tmp »), puis elle prend la place de l'ancien. Si
/// l'appli ou le PC s'arrête en plein milieu, l'ancien fichier reste entier.
pub(crate) fn ecrire_sans_risque(fichier: &Path, contenu: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut nom = fichier.as_os_str().to_owned();
    nom.push(".tmp");
    let copie = PathBuf::from(nom);
    let mut f = std::fs::File::create(&copie)?;
    f.write_all(contenu)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&copie, fichier)
}

/// Écrit un réglage dans son fichier. Si on n'y arrive pas, tant pis : il
/// sera simplement oublié au prochain lancement.
pub(crate) fn memoriser(nom: &str, texte: &str) {
    if let Some(fichier) = fichier_de_reglage(nom) {
        let _ = ecrire_reglage(&fichier, texte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crée un dossier de test vide, propre à chaque test.
    fn dossier_de_test(nom: &str) -> PathBuf {
        let racine = std::env::temp_dir().join(format!("lecteur_mp3_test_{nom}"));
        let _ = std::fs::remove_dir_all(&racine);
        std::fs::create_dir_all(&racine).unwrap();
        racine
    }

    fn noms(chemins: &[PathBuf]) -> Vec<String> {
        chemins.iter().map(|c| nom_du_fichier(c)).collect()
    }

    #[test]
    fn lit_les_sous_dossiers_et_trie() {
        let racine = dossier_de_test("arbre");
        for d in ["Zebda/Album", "abba", "vide/encore vide"] {
            std::fs::create_dir_all(racine.join(d)).unwrap();
        }
        for f in [
            "c.mp3",
            "B.MP3",
            "notes.txt",
            "abba/2.mp3",
            "abba/1.mp3",
            "Zebda/Album/z.mp3",
        ] {
            std::fs::write(racine.join(f), b"").unwrap();
        }

        let contenu = Contenu::lire(&racine);
        assert!(contenu.existe);
        let r = &contenu.racine;
        assert_eq!(r.total, 5);
        // Les dossiers sans MP3 n'apparaissent pas.
        let dossiers: Vec<_> = r.sous_dossiers.iter().map(|d| d.nom.as_str()).collect();
        assert_eq!(dossiers, ["abba", "Zebda"]);
        assert_eq!(noms(&r.morceaux), ["B.MP3", "c.mp3"]);
        assert_eq!(r.sous_dossiers[1].sous_dossiers[0].total, 1);

        // « Tout ajouter » suit l'ordre affiché : sous-dossiers d'abord.
        let mut tous = Vec::new();
        r.tous_les_morceaux(&mut tous);
        assert_eq!(noms(&tous), ["1.mp3", "2.mp3", "z.mp3", "B.MP3", "c.mp3"]);
        let dossiers: Vec<_> = contenu.tous.iter().map(|e| e.dossier.as_str()).collect();
        let album = Path::new("Zebda").join("Album").display().to_string();
        assert_eq!(dossiers, ["abba", "abba", album.as_str(), "", ""]);

        std::fs::remove_dir_all(&racine).unwrap();
    }

    #[test]
    fn dossier_introuvable() {
        let contenu = Contenu::lire(Path::new("ce/dossier/n/existe/pas"));
        assert!(!contenu.existe);
        assert!(contenu.tous.is_empty());
    }

    #[test]
    fn recherche_par_mots_dans_le_nom_et_le_dossier() {
        let racine = dossier_de_test("recherche");
        std::fs::create_dir_all(racine.join("Daft Punk/Discovery")).unwrap();
        for f in [
            "Daft Punk/Discovery/One More Time.mp3",
            "Daft Punk/Around.mp3",
            "one.mp3",
        ] {
            std::fs::write(racine.join(f), b"").unwrap();
        }
        let contenu = Contenu::lire(&racine);
        let trouves = |recherche: &str| -> Vec<String> {
            chercher(&contenu.tous, recherche, |_| "")
                .into_iter()
                .map(|n| nom_du_fichier(&contenu.tous[n].chemin))
                .collect()
        };
        assert_eq!(trouves("daft DISCOVERY"), ["One More Time.mp3"]);
        assert_eq!(trouves("one"), ["One More Time.mp3", "one.mp3"]);
        assert_eq!(trouves("punk"), ["One More Time.mp3", "Around.mp3"]);
        assert!(trouves("beatles").is_empty());
        assert!(trouves("mp3").is_empty());
        // Le nom du dossier de musique lui-même ne compte pas.
        assert!(trouves("lecteur_mp3_test").is_empty());
        std::fs::remove_dir_all(&racine).unwrap();
    }

    #[test]
    fn recherche_aussi_dans_les_tags() {
        let racine = dossier_de_test("recherche_tags");
        for f in ["01.mp3", "02.mp3"] {
            std::fs::write(racine.join(f), b"").unwrap();
        }
        let contenu = Contenu::lire(&racine);
        // Seul 01.mp3 a des tags.
        let tags = |chemin: &Path| -> &str {
            if chemin.ends_with("01.mp3") {
                "one more time daft punk discovery"
            } else {
                ""
            }
        };
        let trouves = |recherche: &str| -> Vec<String> {
            chercher(&contenu.tous, recherche, tags)
                .into_iter()
                .map(|n| nom_du_fichier(&contenu.tous[n].chemin))
                .collect()
        };
        assert_eq!(trouves("Daft time"), ["01.mp3"]);
        // Un mot dans le nom du fichier, l'autre dans les tags.
        assert_eq!(trouves("01 discovery"), ["01.mp3"]);
        assert_eq!(trouves("0"), ["01.mp3", "02.mp3"]);
        assert!(trouves("beatles").is_empty());
        std::fs::remove_dir_all(&racine).unwrap();
    }

    #[test]
    fn memorise_le_dossier() {
        let racine = dossier_de_test("reglage");
        let fichier = racine.join("sous dossier").join("dossier_musique.txt");
        assert_eq!(lire_reglage(&fichier), None);
        let dossier = PathBuf::from(r"C:\Users\Luce\Music\Électro");
        ecrire_reglage(&fichier, &dossier.to_string_lossy()).unwrap();
        assert_eq!(lire_reglage(&fichier), Some(dossier));
        std::fs::write(&fichier, "  \n").unwrap();
        assert_eq!(lire_reglage(&fichier), None);
        std::fs::remove_dir_all(&racine).unwrap();
    }

    /// Appuyer sur le bouton de gauche (0) ou de droite (1), ou le relâcher.
    enum Geste {
        Appui(usize),
        Relache(usize),
    }
    use Geste::{Appui, Relache};

    /// Simule des images successives (à un instant donné, avec des gestes de
    /// souris) devant deux boutons, et compte les clics qui comptent pour chacun.
    fn compter_clics(images: &[(f64, Vec<Geste>)]) -> [usize; 2] {
        let ctx = egui::Context::default();
        let mut comptes = [0, 0];
        let mut image = |temps: f64, evenements: Vec<egui::Event>| {
            let mut positions = [egui::Pos2::ZERO; 2];
            let entree = egui::RawInput {
                time: Some(temps),
                events: evenements,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(entree, |ctx| {
                dater_le_dernier_clic(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        for (n, nom) in ["gauche", "droite"].into_iter().enumerate() {
                            let bouton = ui.button(nom);
                            positions[n] = bouton.rect.center();
                            if clic_simple(ui, &bouton) {
                                comptes[n] += 1;
                            }
                        }
                    });
                });
            });
            positions
        };
        let positions = image(0.0, vec![]); // pour placer les boutons
        for (temps, gestes) in images {
            let evenements = gestes
                .iter()
                .flat_map(|geste| {
                    let (n, pressed) = match geste {
                        Appui(n) => (*n, true),
                        Relache(n) => (*n, false),
                    };
                    let pos = positions[n];
                    [
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                })
                .collect();
            image(*temps, evenements);
        }
        comptes
    }

    /// Des clics (instant, bouton) quand la fenêtre ne se fige pas : après
    /// chaque clic, egui redessine aussitôt une image.
    fn clics(liste: &[(f64, usize)]) -> Vec<(f64, Vec<Geste>)> {
        liste
            .iter()
            .flat_map(|&(temps, n)| [(temps, vec![Appui(n), Relache(n)]), (temps + 0.005, vec![])])
            .collect()
    }

    /// Une grosse bibliothèque : seules les lignes à l'écran sont dessinées.
    /// Après avoir fait défiler loin avec la molette, un clic sur une ligne
    /// ajoute bien le morceau qu'on voit à cet endroit.
    #[test]
    fn une_grosse_bibliotheque_se_parcourt_et_se_clique() {
        let racine = dossier_de_test("grosse");
        for i in 0..2000 {
            std::fs::write(racine.join(format!("{i:04}.mp3")), b"").unwrap();
        }
        let ctx = egui::Context::default();
        let tags = TagsConnus::new(&ctx);
        let mut bibliotheque = Bibliotheque {
            dossier: racine.clone(),
            contenu: Some(Contenu::lire(&racine)),
            en_cours: None,
            recherche: String::new(),
            resultats: Vec::new(),
            recherche_faite: None,
            favoris: HashSet::new(),
            hauteur_ligne: None,
            hauteur_entete: None,
            vue: Vue::Dossiers,
            regroupement: None,
        };
        let dans_la_liste = HashSet::new();
        let mut image = |temps: f64, events: Vec<egui::Event>| {
            let entree = egui::RawInput {
                time: Some(temps),
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            let mut ajoutes = Vec::new();
            let _ = ctx.run(entree, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ajoutes = bibliotheque
                        .afficher(ui, &dans_la_liste, &tags, &[])
                        .a_ajouter;
                });
            });
            ajoutes
        };
        let souris = egui::pos2(100.0, 150.0);
        let mut t = 0.0;
        image(t, vec![]);
        // Dix crans de molette vers le bas, puis le temps que ça défile.
        for _ in 0..10 {
            t += 0.05;
            image(
                t,
                vec![
                    egui::Event::PointerMoved(souris),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Page,
                        delta: egui::vec2(0.0, -1.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        for _ in 0..40 {
            t += 0.05;
            image(t, vec![]);
        }
        let clic = |pressed| egui::Event::PointerButton {
            pos: souris,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        t += 1.0;
        image(t, vec![egui::Event::PointerMoved(souris), clic(true)]);
        let ajoutes = image(t + 0.05, vec![clic(false)]);
        assert_eq!(ajoutes.len(), 1);
        let numero: usize = ajoutes[0]
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(numero > 50, "morceau {numero} : la liste n'a pas défilé");
        std::fs::remove_dir_all(&racine).unwrap();
    }

    /// Vue Albums d'une grosse bibliothèque (2000 albums d'un morceau) :
    /// une fois les tags lus, tout est rangé ; seuls les albums à l'écran
    /// sont dessinés, et après avoir fait défiler loin, on déplie bien
    /// l'album qu'on voit, puis un clic sur son morceau l'ajoute.
    #[test]
    fn vue_albums_d_une_grosse_bibliotheque() {
        use crate::tags::tests::ecrire_mp3_avec;
        let racine = dossier_de_test("grosse_albums");
        for i in 0..2000 {
            let nom = format!("{i:04}.mp3");
            let fichier = ecrire_mp3_avec(
                &format!("album_{nom}"),
                &[
                    ("TIT2", &nom),
                    ("TPE1", "Moi"),
                    ("TALB", &format!("Album {i:04}")),
                ],
                false,
                1,
            );
            std::fs::rename(fichier, racine.join(nom)).unwrap();
        }
        let ctx = egui::Context::default();
        let tags = TagsConnus::new(&ctx);
        let contenu = Contenu::lire(&racine);
        tags.demander(contenu.tous.iter().map(|e| e.chemin.as_path()));
        let mut bibliotheque = Bibliotheque {
            dossier: racine.clone(),
            contenu: Some(contenu),
            en_cours: None,
            recherche: String::new(),
            resultats: Vec::new(),
            recherche_faite: None,
            favoris: HashSet::new(),
            hauteur_ligne: None,
            hauteur_entete: None,
            vue: Vue::Albums,
            regroupement: None,
        };
        let dans_la_liste = HashSet::new();
        let mut image = |temps: f64, events: Vec<egui::Event>| {
            let entree = egui::RawInput {
                time: Some(temps),
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            let mut ajoutes = Vec::new();
            let _ = ctx.run(entree, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ajoutes = bibliotheque
                        .afficher(ui, &dans_la_liste, &tags, &[])
                        .a_ajouter;
                });
            });
            let rangement = bibliotheque
                .regroupement
                .as_ref()
                .map(|(r, _)| (r.albums.len(), r.pas_encore_lus.len()));
            (ajoutes, rangement, bibliotheque.hauteur_entete)
        };
        // On attend que tous les tags soient lus.
        let mut t = 0.0;
        let debut = std::time::Instant::now();
        loop {
            t += 0.05;
            if image(t, vec![]).1 == Some((2000, 0)) {
                break;
            }
            assert!(debut.elapsed().as_secs() < 60, "tags jamais tous lus");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let souris = egui::pos2(100.0, 150.0);
        for _ in 0..10 {
            t += 0.05;
            image(
                t,
                vec![
                    egui::Event::PointerMoved(souris),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Page,
                        delta: egui::vec2(0.0, -1.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        for _ in 0..40 {
            t += 0.05;
            image(t, vec![]);
        }
        let clic = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        // Un clic sur le nom de l'album le déplie (sans rien ajouter)…
        t += 1.0;
        image(
            t,
            vec![egui::Event::PointerMoved(souris), clic(souris, true)],
        );
        let (ajoutes, _, hauteur_entete) = image(t + 0.05, vec![clic(souris, false)]);
        assert!(ajoutes.is_empty());
        for _ in 0..20 {
            t += 0.05;
            image(t, vec![]);
        }
        // … et son morceau apparaît juste dessous.
        let dessous = souris + egui::vec2(0.0, hauteur_entete.unwrap());
        t += 1.0;
        image(
            t,
            vec![egui::Event::PointerMoved(dessous), clic(dessous, true)],
        );
        let (ajoutes, _, _) = image(t + 0.05, vec![clic(dessous, false)]);
        assert_eq!(ajoutes.len(), 1, "aucun morceau sous l'album déplié");
        let numero: usize = ajoutes[0]
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(numero > 50, "morceau {numero} : la liste n'a pas défilé");
        std::fs::remove_dir_all(&racine).unwrap();
    }

    #[test]
    fn retrouve_la_vue_memorisee() {
        for vue in Vue::TOUTES {
            assert_eq!(Vue::depuis_le_code(vue.code()), vue);
        }
        assert_eq!(Vue::depuis_le_code("albums\n"), Vue::Albums);
        assert_eq!(Vue::depuis_le_code("n'importe quoi"), Vue::Dossiers);
    }

    #[test]
    fn un_double_clic_ne_compte_qu_une_fois() {
        // Double-clic, puis triple-clic, sur le même bouton : 1 chacun.
        assert_eq!(compter_clics(&clics(&[(1.0, 0), (1.1, 0)])), [1, 0]);
        assert_eq!(
            compter_clics(&clics(&[(1.0, 0), (1.1, 0), (1.2, 0)])),
            [1, 0]
        );
        // Deux clics espacés sur le même bouton : 2.
        assert_eq!(compter_clics(&clics(&[(1.0, 0), (2.0, 0)])), [2, 0]);
        // Deux clics rapides sur deux boutons différents : 1 chacun.
        assert_eq!(compter_clics(&clics(&[(1.0, 0), (1.1, 1)])), [1, 1]);
    }

    #[test]
    fn double_clic_pendant_que_la_fenetre_est_figee() {
        // Le 1er clic lance la lecture et fige la fenêtre 2 secondes : le
        // 2e clic, fait pendant ce temps, n'est vu qu'à l'image d'après.
        let mut images = vec![
            (1.0, vec![Appui(0)]),
            (1.05, vec![Relache(0)]), // ce clic fige la fenêtre jusqu'à 3.05
            (3.05, vec![Appui(0), Relache(0)]),
            (3.055, vec![]),
        ];
        assert_eq!(compter_clics(&images), [1, 0]);
        // Un vrai nouveau clic, plus tard, compte bien.
        images.extend(clics(&[(4.0, 0)]));
        assert_eq!(compter_clics(&images), [2, 0]);
    }

    #[test]
    fn les_favoris_passent_en_tete_dans_l_ordre() {
        let chemins: Vec<PathBuf> = ["a", "b", "c", "d"].iter().map(PathBuf::from).collect();
        let favoris: HashSet<PathBuf> = ["d", "b"].iter().map(PathBuf::from).collect();
        let tries = favoris_d_abord(&chemins, PathBuf::as_path, &favoris);
        assert_eq!(
            noms(&tries.into_iter().cloned().collect::<Vec<_>>()),
            ["b", "d", "a", "c"]
        );
        let aucun = favoris_d_abord(&chemins, PathBuf::as_path, &HashSet::new());
        assert_eq!(aucun.len(), 4);
        assert_eq!(aucun[0], &chemins[0]);
    }

    #[test]
    fn memorise_les_favoris() {
        let racine = dossier_de_test("favoris");
        let fichier = racine.join("favoris.txt");
        assert!(lire_favoris(&fichier).is_empty());
        let favoris: HashSet<PathBuf> = [
            r"C:\Users\Luce\Music\Zebda",
            r"C:\Users\Luce\Music\Daft Punk\One More Time.mp3",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        ecrire_reglage(&fichier, &texte_des_favoris(&favoris)).unwrap();
        assert_eq!(lire_favoris(&fichier), favoris);
        ecrire_reglage(&fichier, &texte_des_favoris(&HashSet::new())).unwrap();
        assert!(lire_favoris(&fichier).is_empty());
        std::fs::remove_dir_all(&racine).unwrap();
    }

    #[test]
    fn reconnait_les_mp3() {
        assert!(est_un_mp3(Path::new("a.mp3")));
        assert!(est_un_mp3(Path::new("a.Mp3")));
        assert!(!est_un_mp3(Path::new("a.mp3.txt")));
        assert!(!est_un_mp3(Path::new("mp3")));
    }
}
