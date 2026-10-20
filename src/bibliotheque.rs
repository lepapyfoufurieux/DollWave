//! La bibliothèque : tous les MP3 du dossier de musique, rangés par
//! sous-dossier, qu'on ajoute à la liste de lecture d'un clic.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;

use crate::liste::nom_du_fichier;

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
/// dans leur nom ou dans le nom de leur dossier (sans tenir compte des
/// majuscules). Par exemple « daft discovery » trouve
/// « Daft Punk\Discovery\One More Time.mp3 ».
fn chercher(tous: &[Entree], recherche: &str) -> Vec<usize> {
    let mots: Vec<String> = recherche
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    tous.iter()
        .enumerate()
        .filter(|(_, entree)| mots.iter().all(|mot| entree.texte.contains(mot.as_str())))
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
    /// correspondent : on ne recalcule que si elle a changé.
    resultats: Vec<usize>,
    recherche_faite: Option<String>,
}

impl Bibliotheque {
    /// Reprend le dossier mémorisé la dernière fois, ou à défaut le dossier
    /// Musique de Windows, et commence à le lire.
    pub fn new(ctx: &egui::Context) -> Self {
        let dossier = fichier_de_reglage()
            .and_then(|f| lire_reglage(&f))
            .unwrap_or_else(dossier_par_defaut);
        let mut bibliotheque = Self {
            dossier,
            contenu: None,
            en_cours: None,
            recherche: String::new(),
            resultats: Vec::new(),
            recherche_faite: None,
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
        if let Some(fichier) = fichier_de_reglage() {
            // Si on n'arrive pas à l'écrire, tant pis : on redemandera
            // simplement le dossier au prochain lancement.
            let _ = ecrire_reglage(&fichier, &dossier);
        }
        self.dossier = dossier;
        self.contenu = None;
        self.recherche.clear();
        self.actualiser(ctx);
    }

    /// Dessine la bibliothèque. Renvoie les morceaux sur lesquels on a
    /// cliqué, à ajouter à la liste de lecture (vide si aucun).
    /// `dans_la_liste` : les morceaux déjà dans la liste, marqués d'un ✔.
    pub fn afficher(&mut self, ui: &mut egui::Ui, dans_la_liste: &HashSet<&Path>) -> Vec<PathBuf> {
        let mut a_ajouter = Vec::new();

        // La lecture en arrière-plan est-elle finie ?
        if let Some(reception) = &self.en_cours {
            match reception.try_recv() {
                Ok(contenu) => {
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

        if self.en_cours.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Lecture du dossier de musique…");
            });
            return a_ajouter;
        }
        let Some(contenu) = &self.contenu else {
            return a_ajouter;
        };
        if !contenu.existe {
            ui.label("Ce dossier est introuvable. Choisis ton dossier de musique avec « 📁 Changer de dossier… ».");
            return a_ajouter;
        }
        if contenu.tous.is_empty() {
            ui.label("Aucun fichier MP3 dans ce dossier.");
            return a_ajouter;
        }

        let recherche = self.recherche.trim();
        if recherche.is_empty() {
            ui.horizontal(|ui| {
                ui.label(format!("{} morceaux", contenu.tous.len()));
                if ui
                    .small_button("➕ Tout ajouter")
                    .on_hover_text("Ajouter toute la bibliothèque à la liste de lecture")
                    .clicked()
                {
                    contenu.racine.tous_les_morceaux(&mut a_ajouter);
                }
            });
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    afficher_dossier(ui, &contenu.racine, dans_la_liste, &mut a_ajouter);
                });
            return a_ajouter;
        }

        if self.recherche_faite.as_deref() != Some(recherche) {
            self.resultats = chercher(&contenu.tous, recherche);
            self.recherche_faite = Some(recherche.to_string());
        }
        if self.resultats.is_empty() {
            ui.label("Aucun morceau ne correspond à cette recherche.");
            return a_ajouter;
        }
        ui.horizontal(|ui| {
            let nombre = self.resultats.len();
            ui.label(if nombre == 1 {
                "1 résultat".to_string()
            } else {
                format!("{nombre} résultats")
            });
            if ui
                .small_button("➕ Tout ajouter")
                .on_hover_text("Ajouter tous les résultats à la liste de lecture")
                .clicked()
            {
                a_ajouter.extend(
                    self.resultats
                        .iter()
                        .map(|&n| contenu.tous[n].chemin.clone()),
                );
            }
        });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for &n in self.resultats.iter().take(RESULTATS_MAX) {
                    let entree = &contenu.tous[n];
                    ui.horizontal(|ui| {
                        ligne_morceau(ui, &entree.chemin, dans_la_liste, &mut a_ajouter);
                        ui.label(egui::RichText::new(&entree.dossier).small().weak());
                    });
                }
                if self.resultats.len() > RESULTATS_MAX {
                    ui.label(format!(
                        "… et {} autres : précise ta recherche pour les voir.",
                        self.resultats.len() - RESULTATS_MAX
                    ));
                }
            });
        a_ajouter
    }
}

/// Dessine le contenu d'un dossier : ses sous-dossiers, qu'on peut déplier
/// (fermés au départ), puis ses morceaux.
fn afficher_dossier(
    ui: &mut egui::Ui,
    dossier: &Dossier,
    dans_la_liste: &HashSet<&Path>,
    a_ajouter: &mut Vec<PathBuf>,
) {
    for sous_dossier in &dossier.sous_dossiers {
        let id = ui.make_persistent_id(&sous_dossier.chemin);
        let mut basculer = false;
        let mut entete =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    // Un clic sur le nom du dossier le déplie ou le replie, comme la flèche.
                    let nom = format!("📁 {} ({})", sous_dossier.nom, sous_dossier.total);
                    if ui
                        .add(egui::Label::new(nom).sense(egui::Sense::click()))
                        .clicked()
                    {
                        basculer = true;
                    }
                    if ui
                        .small_button("➕ tout")
                        .on_hover_text("Ajouter tout ce dossier à la liste de lecture")
                        .clicked()
                    {
                        sous_dossier.tous_les_morceaux(a_ajouter);
                    }
                });
        if basculer {
            entete.toggle();
        }
        entete.body(|ui| afficher_dossier(ui, sous_dossier, dans_la_liste, a_ajouter));
    }
    for morceau in &dossier.morceaux {
        ligne_morceau(ui, morceau, dans_la_liste, a_ajouter);
    }
}

/// Une ligne pour un morceau : un clic l'ajoute à la liste de lecture.
fn ligne_morceau(
    ui: &mut egui::Ui,
    chemin: &Path,
    dans_la_liste: &HashSet<&Path>,
    a_ajouter: &mut Vec<PathBuf>,
) {
    let deja = dans_la_liste.contains(chemin);
    let texte = format!(
        "{} {}",
        if deja { "✔" } else { "➕" },
        nom_du_fichier(chemin)
    );
    let aide = if deja {
        "Déjà dans la liste de lecture (cliquer pour l'ajouter encore une fois)"
    } else {
        "Cliquer pour ajouter à la liste de lecture"
    };
    if ui
        .selectable_label(false, texte)
        .on_hover_text(aide)
        .clicked()
    {
        a_ajouter.push(chemin.to_path_buf());
    }
}

/// Le fichier se termine-t-il par .mp3 (ou .MP3, .Mp3…) ?
fn est_un_mp3(chemin: &Path) -> bool {
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

/// Le petit fichier où l'on mémorise le dossier de musique choisi :
/// %APPDATA%\lecteur_mp3\dossier_musique.txt.
fn fichier_de_reglage() -> Option<PathBuf> {
    let base = variable("APPDATA")
        .map(PathBuf::from)
        .or_else(|| variable("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("lecteur_mp3").join("dossier_musique.txt"))
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

/// Mémorise le dossier choisi (en créant le dossier du réglage si besoin).
fn ecrire_reglage(fichier: &Path, dossier: &Path) -> std::io::Result<()> {
    if let Some(parent) = fichier.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(fichier, dossier.to_string_lossy().as_bytes())
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
            chercher(&contenu.tous, recherche)
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
    fn memorise_le_dossier() {
        let racine = dossier_de_test("reglage");
        let fichier = racine.join("sous dossier").join("dossier_musique.txt");
        assert_eq!(lire_reglage(&fichier), None);
        let dossier = PathBuf::from(r"C:\Users\Luce\Music\Électro");
        ecrire_reglage(&fichier, &dossier).unwrap();
        assert_eq!(lire_reglage(&fichier), Some(dossier));
        std::fs::write(&fichier, "  \n").unwrap();
        assert_eq!(lire_reglage(&fichier), None);
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
