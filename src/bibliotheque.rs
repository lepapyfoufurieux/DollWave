//! La bibliothèque : tous les MP3 du dossier de musique, rangés par
//! sous-dossier, qu'on ajoute à la liste de lecture d'un clic.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;

use crate::liste::{nom_du_fichier, rangee};
use crate::playlists::{menu_ajouter_a_une_playlist, MorceauxGlisses};
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
        dossier.sous_dossiers.sort_by_key(|d| d.nom.to_lowercase());
        dossier
            .morceaux
            .sort_by_key(|m| nom_du_fichier(m).to_lowercase());
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
fn chercher(tous: &[Entree], recherche: &str, tags: impl Fn(&Path) -> String) -> Vec<usize> {
    let mots: Vec<String> = recherche
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    tous.iter()
        .enumerate()
        .filter(|(_, entree)| {
            let tags = tags(&entree.chemin);
            mots.iter()
                .all(|mot| entree.texte.contains(mot.as_str()) || tags.contains(mot.as_str()))
        })
        .map(|(n, _)| n)
        .collect()
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
}

impl Bibliotheque {
    /// Reprend le dossier mémorisé la dernière fois, ou à défaut le dossier
    /// Musique de Windows, et commence à le lire.
    pub fn new(ctx: &egui::Context) -> Self {
        let dossier = fichier_de_reglage(REGLAGE_DOSSIER)
            .and_then(|f| lire_reglage(&f))
            .unwrap_or_else(dossier_par_defaut);
        let favoris = fichier_de_reglage(REGLAGE_FAVORIS)
            .map(|f| lire_favoris(&f))
            .unwrap_or_default();
        let mut bibliotheque = Self {
            dossier,
            favoris,
            contenu: None,
            en_cours: None,
            recherche: String::new(),
            resultats: Vec::new(),
            recherche_faite: None,
            hauteur_ligne: None,
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
            hauteur: self.hauteur_ligne,
        };
        self.afficher_contenu(ui, &mut lignes);
        let Lignes {
            a_ajouter,
            etoile,
            vers_playlist,
            hauteur,
            ..
        } = lignes;
        self.hauteur_ligne = hauteur;
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
        }
    }

    /// Dessine les morceaux : l'arbre des dossiers, ou les résultats de la
    /// recherche.
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
            let tags = lignes.tags;
            self.resultats = chercher(&contenu.tous, recherche, |m| tags.texte_a_chercher(m));
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
    /// La hauteur d'une ligne de morceau (voir `liste::rangee`).
    hauteur: Option<f32>,
}

/// Quand on commence à faire glisser un dossier ou un morceau, on emporte
/// ses morceaux, pour les lâcher sur une playlist.
fn glisser(reponse: &egui::Response, morceaux: impl FnOnce() -> Vec<PathBuf>) {
    if reponse.drag_started() {
        egui::DragAndDrop::set_payload(&reponse.ctx, MorceauxGlisses(morceaux()));
    }
}

/// Le menu du clic droit sur un dossier ou un morceau : « Ajouter à la
/// playlist » puis le nom de chaque playlist. `morceaux` donne ce qu'il
/// faut y ajouter (on ne le calcule que si on a choisi une playlist).
fn menu_playlist(
    reponse: &egui::Response,
    lignes: &mut Lignes,
    morceaux: impl FnOnce() -> Vec<PathBuf>,
) {
    if let Some(nom) = menu_ajouter_a_une_playlist(reponse, lignes.playlists) {
        lignes.vers_playlist = Some((nom, morceaux()));
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
        let mut basculer = false;
        let mut entete =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    etoile(ui, &sous_dossier.chemin, lignes);
                    // Un clic sur le nom du dossier le déplie ou le replie, comme la flèche.
                    let nom = format!("📁 {} ({})", sous_dossier.nom, sous_dossier.total);
                    let etiquette = ui.add(
                        egui::Label::new(nom)
                            .selectable(false)
                            .sense(egui::Sense::click_and_drag()),
                    );
                    if clic_simple(ui, &etiquette) {
                        basculer = true;
                    }
                    glisser(&etiquette, || {
                        let mut morceaux = Vec::new();
                        sous_dossier.tous_les_morceaux(&mut morceaux);
                        morceaux
                    });
                    menu_playlist(&etiquette, lignes, || {
                        let mut morceaux = Vec::new();
                        sous_dossier.tous_les_morceaux(&mut morceaux);
                        morceaux
                    });
                    let bouton = ui
                        .small_button("➕ tout")
                        .on_hover_text("Ajouter tout ce dossier à la liste de lecture");
                    if clic_simple(ui, &bouton) {
                        // Dans l'ordre alphabétique, sans faire passer les
                        // favoris devant.
                        sous_dossier.tous_les_morceaux(&mut lignes.a_ajouter);
                    }
                });
        if basculer {
            entete.toggle();
        }
        entete.body(|ui| afficher_dossier(ui, sous_dossier, lignes));
    }
    let mut hauteur = lignes.hauteur;
    for morceau in favoris_d_abord(&dossier.morceaux, PathBuf::as_path, lignes.favoris) {
        rangee(ui, &mut hauteur, morceau, |ui| {
            ligne_morceau(ui, morceau, lignes)
        });
    }
    lignes.hauteur = hauteur;
}

/// L'étoile devant un dossier ou un morceau : ★ s'il est en favori, ☆ sinon.
/// Un clic la change.
fn etoile(ui: &mut egui::Ui, chemin: &Path, lignes: &mut Lignes) {
    let favori = lignes.favoris.contains(chemin);
    let (texte, aide) = if favori {
        (
            egui::RichText::new("★").color(egui::Color32::GOLD),
            "Retirer des favoris",
        )
    } else {
        (
            egui::RichText::new("☆").weak(),
            "Mettre en favori (remonte en tête de son dossier)",
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
    menu_playlist(&ligne, lignes, || vec![chemin.to_path_buf()]);
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

/// Le dossier Musique de Windows : C:\Users\<nom>\Music.
fn dossier_par_defaut() -> PathBuf {
    variable("USERPROFILE")
        .or_else(|| variable("HOME"))
        .map(|maison| PathBuf::from(maison).join("Music"))
        .unwrap_or_default()
}

/// Les petits fichiers où l'on mémorise nos réglages.
const REGLAGE_DOSSIER: &str = "dossier_musique.txt";
const REGLAGE_FAVORIS: &str = "favoris.txt";

/// Un petit fichier de réglage, par exemple
/// %APPDATA%\lecteur_mp3\dossier_musique.txt.
pub(crate) fn fichier_de_reglage(nom: &str) -> Option<PathBuf> {
    let base = variable("APPDATA")
        .map(PathBuf::from)
        .or_else(|| variable("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("lecteur_mp3").join(nom))
}

/// La valeur d'une variable d'environnement de Windows, si elle n'est pas vide.
fn variable(nom: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(nom).filter(|v| !v.is_empty())
}

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
    std::fs::write(fichier, texte)
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
            chercher(&contenu.tous, recherche, |_| String::new())
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
        let tags = |chemin: &Path| -> String {
            if chemin.ends_with("01.mp3") {
                "one more time daft punk discovery".to_string()
            } else {
                String::new()
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
