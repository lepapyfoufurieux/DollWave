//! Les playlists : des listes de lecture enregistrées, chacune dans un
//! fichier .m3u du dossier %APPDATA%\lecteur_mp3\playlists. Le .m3u est un
//! simple fichier texte (un morceau par ligne) que VLC ou Winamp savent
//! aussi ouvrir.

use std::io;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::bibliotheque::{clic_simple, fichier_de_reglage};

/// Les caractères que Windows refuse dans un nom de fichier.
const INTERDITS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Une playlist enregistrée : son nom et son nombre de morceaux.
pub struct Playlist {
    pub nom: String,
    pub nombre: usize,
}

/// Des morceaux qu'on fait glisser depuis la bibliothèque, à lâcher sur
/// une playlist pour les y ajouter.
pub struct MorceauxGlisses(pub Vec<PathBuf>);

/// Pendant qu'on fait glisser des morceaux, une étiquette suit la souris
/// pour montrer ce qu'on transporte.
pub fn dessiner_glissement(ctx: &egui::Context) {
    let Some(glisses) = egui::DragAndDrop::payload::<MorceauxGlisses>(ctx) else {
        return;
    };
    let Some(souris) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let texte = match glisses.0.len() {
        1 => "🎵 1 morceau : lâche-le sur une playlist".to_string(),
        n => format!("🎵 {n} morceaux : lâche-les sur une playlist"),
    };
    egui::Area::new(egui::Id::new("morceaux_glisses"))
        .order(egui::Order::Tooltip)
        .fixed_pos(souris + egui::vec2(14.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(texte);
            });
        });
}

/// Ce que l'utilisateur a demandé en cliquant sur une playlist.
pub enum Demande {
    /// Remplacer la liste de lecture par ces morceaux (clic sur une playlist).
    Charger(Vec<PathBuf>),
    /// Ajouter ces morceaux à la suite de la liste de lecture (bouton ➕).
    AjouterALaListe(Vec<PathBuf>),
}

/// Le nom qu'on est en train de taper.
enum Saisie {
    /// Pour créer une nouvelle playlist (vide).
    Nouvelle(String),
    /// Pour renommer la playlist `ancien`.
    Renommer { ancien: String, texte: String },
}

pub struct Playlists {
    /// Le dossier des fichiers .m3u (rien si on ne sait pas où le mettre).
    dossier: Option<PathBuf>,
    /// Les playlists trouvées dans le dossier, triées par nom.
    toutes: Vec<Playlist>,
    /// La playlist chargée dans la liste de lecture : « 💾 Enregistrer »
    /// la met à jour.
    chargee: Option<String>,
    saisie: Option<Saisie>,
    /// La playlist dont on demande confirmation avant de la supprimer.
    a_supprimer: Option<String>,
    /// Le champ de saisie vient d'apparaître : on y met le curseur.
    donner_la_main: bool,
    /// Un message affiché sous les playlists (erreur ou confirmation).
    message: String,
}

impl Playlists {
    pub fn new() -> Self {
        Self::dans(fichier_de_reglage("playlists"))
    }

    fn dans(dossier: Option<PathBuf>) -> Self {
        let mut playlists = Self {
            dossier,
            toutes: Vec::new(),
            chargee: None,
            saisie: None,
            a_supprimer: None,
            donner_la_main: false,
            message: String::new(),
        };
        playlists.actualiser();
        playlists
    }

    /// Relit le dossier des playlists.
    fn actualiser(&mut self) {
        self.toutes = self.dossier.as_deref().map(lister).unwrap_or_default();
    }

    /// Les noms des playlists, dans l'ordre affiché.
    pub fn noms(&self) -> Vec<String> {
        self.toutes.iter().map(|p| p.nom.clone()).collect()
    }

    /// La playlist chargée dans la liste de lecture, s'il y en a une.
    pub fn chargee(&self) -> Option<&str> {
        self.chargee.as_deref()
    }

    /// La liste de lecture a été vidée : elle n'est plus liée à une playlist.
    pub fn oublier_la_chargee(&mut self) {
        self.chargee = None;
    }

    /// Ouvre le champ pour taper le nom d'une nouvelle playlist.
    fn demander_un_nom(&mut self) {
        self.saisie = Some(Saisie::Nouvelle(String::new()));
        self.donner_la_main = true;
        self.a_supprimer = None;
        self.message.clear();
    }

    /// Ajoute des morceaux à la fin d'une playlist (clic droit dans la bibliothèque).
    pub fn ajouter_a(&mut self, nom: &str, morceaux: &[PathBuf]) {
        let Some(dossier) = self.dossier.clone() else {
            return;
        };
        self.message = match lire(&dossier, nom).and_then(|mut contenu| {
            contenu.extend_from_slice(morceaux);
            ecrire(&dossier, nom, &contenu)
        }) {
            Ok(()) if morceaux.len() == 1 => format!("1 morceau ajouté à « {nom} »."),
            Ok(()) => format!("{} morceaux ajoutés à « {nom} ».", morceaux.len()),
            Err(e) => format!("Impossible de modifier « {nom} » : {e}"),
        };
        self.actualiser();
    }

    /// Dessine les playlists.
    pub fn afficher(&mut self, ui: &mut egui::Ui) -> Option<Demande> {
        let mut demande = None;

        ui.horizontal(|ui| {
            ui.strong("Playlists");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("➕ Nouvelle")
                    .on_hover_text("Créer une playlist vide, à remplir en y glissant des morceaux")
                    .clicked()
                {
                    self.demander_un_nom();
                }
            });
        });

        if matches!(self.saisie, Some(Saisie::Nouvelle(_))) {
            self.champ_de_saisie(ui);
        }

        if self.dossier.is_none() {
            ui.label("Impossible de trouver où enregistrer les playlists.");
        } else if self.toutes.is_empty() && self.saisie.is_none() {
            ui.weak("Aucune playlist pour l'instant. Crée-en une avec « ➕ Nouvelle », puis glisses-y des morceaux depuis la bibliothèque.");
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height((ui.available_height() - 24.0).max(40.0))
            .show(ui, |ui| {
                for n in 0..self.toutes.len() {
                    if let Some(d) = self.ligne(ui, n) {
                        demande = Some(d);
                    }
                }
            });

        if !self.message.is_empty() {
            ui.label(egui::RichText::new(&self.message).small().weak());
        }
        demande
    }

    /// Une ligne de playlist : son nom (un clic la charge), et ses boutons.
    fn ligne(&mut self, ui: &mut egui::Ui, n: usize) -> Option<Demande> {
        let nom = self.toutes[n].nom.clone();

        // En train de la renommer : le champ prend la place de la ligne.
        if matches!(&self.saisie, Some(Saisie::Renommer { ancien, .. }) if *ancien == nom) {
            self.champ_de_saisie(ui);
            return None;
        }

        // On a cliqué sur 🗑 : on demande confirmation.
        if self.a_supprimer.as_deref() == Some(nom.as_str()) {
            ui.horizontal(|ui| {
                ui.label(format!("Supprimer « {nom} » ?"));
                if ui.small_button("Oui").clicked() {
                    self.supprimer(&nom);
                }
                if ui.small_button("Non").clicked() {
                    self.a_supprimer = None;
                }
            });
            return None;
        }

        let mut demande = None;
        let rangee = ui.horizontal(|ui| {
            let chargee = self.chargee.as_deref() == Some(nom.as_str());
            let texte = format!("🎵 {} ({})", nom, self.toutes[n].nombre);
            let etiquette = ui.selectable_label(chargee, texte).on_hover_text(
                "Cliquer pour écouter cette playlist (remplace la liste de lecture)",
            );
            if clic_simple(ui, &etiquette) {
                if let Some(fichiers) = self.lire_pour(&nom) {
                    self.chargee = Some(nom.clone());
                    demande = Some(Demande::Charger(fichiers));
                }
            }
            // Les boutons, calés à droite.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("🗑").on_hover_text("Supprimer").clicked() {
                    self.a_supprimer = Some(nom.clone());
                    self.saisie = None;
                }
                if ui.small_button("✏").on_hover_text("Renommer").clicked() {
                    self.saisie = Some(Saisie::Renommer {
                        ancien: nom.clone(),
                        texte: nom.clone(),
                    });
                    self.donner_la_main = true;
                    self.a_supprimer = None;
                }
                let bouton = ui
                    .small_button("➕")
                    .on_hover_text("Ajouter à la suite de la liste de lecture");
                if clic_simple(ui, &bouton) {
                    demande = self.lire_pour(&nom).map(Demande::AjouterALaListe);
                }
            });
        });

        // Des morceaux glissés depuis la bibliothèque survolent la ligne : on
        // l'encadre, et s'ils sont lâchés ici, on les ajoute à la playlist.
        let ctx = ui.ctx().clone();
        if egui::DragAndDrop::has_payload_of_type::<MorceauxGlisses>(&ctx)
            && ui.rect_contains_pointer(rangee.response.rect)
        {
            ui.painter().rect_stroke(
                rangee.response.rect.expand(2.0),
                3.0,
                ui.visuals().selection.stroke,
            );
            if ctx.input(|i| i.pointer.any_released()) {
                if let Some(glisses) = egui::DragAndDrop::take_payload::<MorceauxGlisses>(&ctx) {
                    self.ajouter_a(&nom, &glisses.0);
                }
            }
        }
        demande
    }

    /// Lit une playlist pour l'écouter. Les morceaux qui n'existent plus
    /// (déplacés, supprimés) sont laissés de côté, en le signalant.
    fn lire_pour(&mut self, nom: &str) -> Option<Vec<PathBuf>> {
        let dossier = self.dossier.as_deref()?;
        match lire(dossier, nom) {
            Ok(fichiers) => {
                let total = fichiers.len();
                let presents: Vec<PathBuf> = fichiers.into_iter().filter(|f| f.is_file()).collect();
                self.message = match total - presents.len() {
                    0 => String::new(),
                    1 => "1 morceau de cette playlist est introuvable : il a été ignoré.".to_string(),
                    manquants => format!(
                        "{manquants} morceaux de cette playlist sont introuvables : ils ont été ignorés."
                    ),
                };
                Some(presents)
            }
            Err(e) => {
                self.message = format!("Impossible de lire « {nom} » : {e}");
                None
            }
        }
    }

    /// Le champ où l'on tape un nom (nouvelle playlist ou renommage).
    /// Entrée valide, Échap annule.
    fn champ_de_saisie(&mut self, ui: &mut egui::Ui) {
        let Some(saisie) = &mut self.saisie else {
            return;
        };
        let texte = match saisie {
            Saisie::Nouvelle(texte) | Saisie::Renommer { texte, .. } => texte,
        };
        let mut valider = false;
        let mut annuler = false;
        ui.horizontal(|ui| {
            let champ = ui.add(
                egui::TextEdit::singleline(texte)
                    .id(egui::Id::new("nom_de_playlist"))
                    .hint_text("Nom de la playlist")
                    .desired_width((ui.available_width() - 80.0).max(60.0)),
            );
            if self.donner_la_main {
                champ.request_focus();
                self.donner_la_main = false;
            }
            if champ.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                valider = true;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                annuler = true;
            }
            if ui.small_button("✔").on_hover_text("Valider").clicked() {
                valider = true;
            }
            if ui.small_button("✖").on_hover_text("Annuler").clicked() {
                annuler = true;
            }
        });
        if annuler {
            self.saisie = None;
        } else if valider {
            self.valider_la_saisie();
        }
    }

    fn valider_la_saisie(&mut self) {
        let Some(dossier) = self.dossier.clone() else {
            return;
        };
        let resultat = match &self.saisie {
            Some(Saisie::Nouvelle(texte)) => {
                nom_valide(texte).and_then(|nom| creer(&dossier, &nom, &[]).map(|()| (None, nom)))
            }
            Some(Saisie::Renommer { ancien, texte }) => nom_valide(texte).and_then(|nom| {
                renommer(&dossier, ancien, &nom).map(|()| (Some(ancien.clone()), nom))
            }),
            None => return,
        };
        match resultat {
            Ok((ancien, nom)) => {
                self.message = match &ancien {
                    None => format!("Playlist « {nom} » créée."),
                    Some(ancien) => format!("« {ancien} » renommée en « {nom} »."),
                };
                // Si l'on renomme la playlist chargée, elle garde ce lien.
                if ancien.is_some() && ancien == self.chargee {
                    self.chargee = Some(nom);
                }
                self.saisie = None;
                self.actualiser();
            }
            // On garde le champ ouvert pour corriger le nom.
            Err(e) => {
                self.message = e;
                self.donner_la_main = true;
            }
        }
    }

    fn supprimer(&mut self, nom: &str) {
        self.a_supprimer = None;
        let Some(dossier) = self.dossier.as_deref() else {
            return;
        };
        self.message = match std::fs::remove_file(chemin_de(dossier, nom)) {
            Ok(()) => {
                if self.chargee.as_deref() == Some(nom) {
                    self.chargee = None;
                }
                format!("Playlist « {nom} » supprimée.")
            }
            Err(e) => format!("Impossible de supprimer « {nom} » : {e}"),
        };
        self.actualiser();
    }
}

/// Le fichier d'une playlist.
fn chemin_de(dossier: &Path, nom: &str) -> PathBuf {
    dossier.join(format!("{nom}.m3u"))
}

/// Vérifie qu'un nom peut servir de nom de fichier, et le renvoie sans les
/// espaces autour.
fn nom_valide(texte: &str) -> Result<String, String> {
    let nom = texte.trim();
    if nom.is_empty() {
        return Err("Donne un nom à la playlist.".to_string());
    }
    if nom.contains(INTERDITS) || nom.ends_with('.') {
        return Err(
            "Un nom de playlist ne peut pas contenir \\ / : * ? \" < > | ni finir par un point."
                .to_string(),
        );
    }
    Ok(nom.to_string())
}

/// Les playlists du dossier, triées par nom sans tenir compte des majuscules.
fn lister(dossier: &Path) -> Vec<Playlist> {
    let Ok(contenu) = std::fs::read_dir(dossier) else {
        return Vec::new(); // pas encore de dossier : aucune playlist
    };
    let mut toutes: Vec<Playlist> = contenu
        .flatten()
        .map(|entree| entree.path())
        .filter(|chemin| {
            chemin
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("m3u"))
        })
        .filter_map(|chemin| {
            let nom = chemin.file_stem()?.to_string_lossy().into_owned();
            let nombre = std::fs::read_to_string(&chemin)
                .map(|texte| lignes_de_morceaux(&texte).count())
                .unwrap_or(0);
            Some(Playlist { nom, nombre })
        })
        .collect();
    toutes.sort_by_key(|p| p.nom.to_lowercase());
    toutes
}

/// Les lignes d'un .m3u qui désignent un morceau (les autres, qui
/// commencent par #, sont des commentaires ou des informations).
fn lignes_de_morceaux(texte: &str) -> impl Iterator<Item = &str> {
    texte
        .trim_start_matches('\u{feff}') // marque que certains logiciels mettent au début
        .lines()
        .map(str::trim)
        .filter(|ligne| !ligne.is_empty() && !ligne.starts_with('#'))
}

/// Les morceaux d'une playlist, dans l'ordre. Un chemin relatif (écrit
/// par un autre logiciel) part du dossier des playlists.
fn lire(dossier: &Path, nom: &str) -> io::Result<Vec<PathBuf>> {
    let texte = std::fs::read_to_string(chemin_de(dossier, nom))?;
    Ok(lignes_de_morceaux(&texte)
        .map(|ligne| dossier.join(ligne))
        .collect())
}

/// Écrit (ou réécrit) une playlist.
fn ecrire(dossier: &Path, nom: &str, morceaux: &[PathBuf]) -> io::Result<()> {
    std::fs::create_dir_all(dossier)?;
    let mut texte = String::from("#EXTM3U\r\n");
    for morceau in morceaux {
        texte.push_str(&morceau.to_string_lossy());
        texte.push_str("\r\n");
    }
    std::fs::write(chemin_de(dossier, nom), texte)
}

/// Crée une playlist, sans écraser une playlist du même nom.
fn creer(dossier: &Path, nom: &str, morceaux: &[PathBuf]) -> Result<(), String> {
    if existe(dossier, nom) {
        return Err(format!("Une playlist s'appelle déjà « {nom} »."));
    }
    ecrire(dossier, nom, morceaux).map_err(|e| format!("Impossible de créer « {nom} » : {e}"))
}

fn renommer(dossier: &Path, ancien: &str, nouveau: &str) -> Result<(), String> {
    if ancien == nouveau {
        return Ok(());
    }
    // Changer seulement des majuscules (« rock » → « Rock ») est permis :
    // pour Windows, c'est le même fichier.
    if !ancien.eq_ignore_ascii_case(nouveau) && existe(dossier, nouveau) {
        return Err(format!("Une playlist s'appelle déjà « {nouveau} »."));
    }
    std::fs::rename(chemin_de(dossier, ancien), chemin_de(dossier, nouveau))
        .map_err(|e| format!("Impossible de renommer « {ancien} » : {e}"))
}

/// Une playlist de ce nom existe-t-elle déjà ? Comme pour Windows, les
/// majuscules ne comptent pas.
fn existe(dossier: &Path, nom: &str) -> bool {
    lister(dossier)
        .iter()
        .any(|p| p.nom.to_lowercase() == nom.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier_de_test(nom: &str) -> PathBuf {
        let racine = std::env::temp_dir().join(format!("lecteur_mp3_test_playlists_{nom}"));
        let _ = std::fs::remove_dir_all(&racine);
        racine
    }

    fn noms(dossier: &Path) -> Vec<(String, usize)> {
        lister(dossier)
            .into_iter()
            .map(|p| (p.nom, p.nombre))
            .collect()
    }

    #[test]
    fn cree_relit_et_trie() {
        let dossier = dossier_de_test("cree");
        let musique = std::env::temp_dir().join("Musique");
        let morceaux = vec![
            musique.join("tout à la fois satine").join("01.mp3"),
            musique.join("yeule").join("02.mp3"),
        ];
        creer(&dossier, "zen", &morceaux).unwrap();
        creer(&dossier, "Apéro", &morceaux[..1]).unwrap();
        assert_eq!(
            noms(&dossier),
            vec![("Apéro".to_string(), 1), ("zen".to_string(), 2)]
        );
        assert_eq!(lire(&dossier, "zen").unwrap(), morceaux);
        // Le fichier est un vrai .m3u.
        let texte = std::fs::read_to_string(dossier.join("zen.m3u")).unwrap();
        assert!(texte.starts_with("#EXTM3U"));
    }

    #[test]
    fn refuse_un_nom_deja_pris_ou_invalide() {
        let dossier = dossier_de_test("refuse");
        creer(&dossier, "Rock", &[]).unwrap();
        assert!(creer(&dossier, "rock", &[]).is_err());
        assert!(nom_valide("   ").is_err());
        assert!(nom_valide("a/b").is_err());
        assert!(nom_valide("fin.").is_err());
        assert_eq!(nom_valide("  Soirée  ").unwrap(), "Soirée");
    }

    #[test]
    fn renomme_et_supprime() {
        let dossier = dossier_de_test("renomme");
        creer(&dossier, "a", &[PathBuf::from("x.mp3")]).unwrap();
        creer(&dossier, "b", &[]).unwrap();
        assert!(renommer(&dossier, "a", "B").is_err());
        renommer(&dossier, "a", "c").unwrap();
        assert_eq!(
            noms(&dossier),
            vec![("b".to_string(), 0), ("c".to_string(), 1)]
        );
        // Changer seulement les majuscules est permis.
        renommer(&dossier, "c", "C").unwrap();
        let mut playlists = Playlists::dans(Some(dossier.clone()));
        playlists.supprimer("b");
        assert_eq!(playlists.noms(), vec!["C".to_string()]);
    }

    #[test]
    fn ajoute_a_la_fin() {
        let dossier = dossier_de_test("ajoute");
        creer(&dossier, "p", &[PathBuf::from("/m/1.mp3")]).unwrap();
        let mut playlists = Playlists::dans(Some(dossier.clone()));
        playlists.ajouter_a("p", &[PathBuf::from("/m/2.mp3"), PathBuf::from("/m/3.mp3")]);
        assert_eq!(
            lire(&dossier, "p").unwrap(),
            vec![
                PathBuf::from("/m/1.mp3"),
                PathBuf::from("/m/2.mp3"),
                PathBuf::from("/m/3.mp3")
            ]
        );
        assert_eq!(playlists.toutes[0].nombre, 3);
    }

    #[test]
    fn lit_un_m3u_d_un_autre_logiciel() {
        let dossier = dossier_de_test("autre");
        std::fs::create_dir_all(&dossier).unwrap();
        std::fs::write(
            dossier.join("vlc.m3u"),
            "\u{feff}#EXTM3U\n#EXTINF:123,Artiste - Titre\nsous/dossier/1.mp3\n\n/absolu/2.mp3\n",
        )
        .unwrap();
        assert_eq!(
            lire(&dossier, "vlc").unwrap(),
            vec![
                dossier.join("sous/dossier/1.mp3"),
                PathBuf::from("/absolu/2.mp3")
            ]
        );
    }
}
