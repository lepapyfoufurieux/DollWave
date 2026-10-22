//! Les playlists : des listes de lecture enregistrées, chacune dans un
//! fichier .m3u du dossier %APPDATA%\dollwave\playlists. Le .m3u est un
//! simple fichier texte (un morceau par ligne) que VLC ou Winamp savent
//! aussi ouvrir.

use std::io;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::bibliotheque::{clic_simple, ecrire_sans_risque};
use crate::liste::{nom_du_fichier, place_sous_la_souris, MorceauGlisse};
use crate::systeme::fichier_de_reglage;
use crate::tags::TagsConnus;
use crate::theme;

/// Les caractères que Windows refuse dans un nom de fichier.
const INTERDITS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Le nom de la playlist automatique « Les plus écoutés » quand elle est
/// chargée (avec un caractère interdit, pour ne jamais être confondue avec
/// une playlist enregistrée du même nom).
const PLUS_ECOUTES: &str = "\\plus-ecoutes";

/// Une playlist enregistrée : son nom et ses morceaux, dans l'ordre.
pub struct Playlist {
    pub nom: String,
    pub morceaux: Vec<PathBuf>,
}

/// Des morceaux qu'on fait glisser depuis la bibliothèque, à lâcher sur
/// une playlist pour les y ajouter.
pub struct MorceauxGlisses(pub Vec<PathBuf>);

/// Un titre d'une playlist dépliée qu'on fait glisser pour le déplacer
/// dans cette même playlist.
struct TitreGlisse {
    playlist: String,
    depuis: usize,
}

/// Ce qu'on a choisi dans le menu du clic droit sur un morceau.
pub enum ChoixDuMenu {
    /// Ajouter le morceau à cette playlist.
    Playlist(String),
    /// Ouvrir la fenêtre « Modifier les tags ».
    ModifierLesTags,
    /// Ouvrir la fenêtre « Infos du fichier » (pour un morceau seul).
    Infos,
}

/// Le menu du clic droit sur un morceau, un dossier, un artiste ou un album
/// (de la bibliothèque ou de la liste de lecture) : « ℹ Infos du fichier… »
/// si `infos` (un morceau seul), `modifier` (« ✏ Modifier les tags… »), puis
/// « Ajouter à la playlist » et le nom de chaque playlist.
pub fn menu_clic_droit(
    reponse: &egui::Response,
    noms: &[String],
    modifier: &str,
    infos: bool,
) -> Option<ChoixDuMenu> {
    let mut choix = None;
    reponse.context_menu(|ui| {
        if infos && ui.button("ℹ Infos du fichier…").clicked() {
            choix = Some(ChoixDuMenu::Infos);
            ui.close_menu();
        }
        if ui.button(modifier).clicked() {
            choix = Some(ChoixDuMenu::ModifierLesTags);
            ui.close_menu();
        }
        ui.separator();
        let mut choisie = None;
        choisir_une_playlist(ui, noms, &mut choisie);
        if let Some(nom) = choisie {
            choix = Some(ChoixDuMenu::Playlist(nom));
        }
    });
    choix
}

fn choisir_une_playlist(ui: &mut egui::Ui, noms: &[String], choisie: &mut Option<String>) {
    ui.label("Ajouter à la playlist :");
    if noms.is_empty() {
        ui.weak("Aucune playlist : crée-en une avec « ➕ Nouvelle », en bas à gauche.");
    }
    for nom in noms {
        if ui.button(format!("🎵 {nom}")).clicked() {
            *choisie = Some(nom.clone());
            ui.close_menu();
        }
    }
}

/// Pendant qu'on fait glisser des morceaux, une étiquette suit la souris
/// pour montrer ce qu'on transporte.
pub fn dessiner_glissement(ctx: &egui::Context) {
    let texte = if let Some(glisses) = egui::DragAndDrop::payload::<MorceauxGlisses>(ctx) {
        match glisses.0.len() {
            1 => "🎵 1 morceau : lâche-le sur une playlist".to_string(),
            n => format!("🎵 {n} morceaux : lâche-les sur une playlist"),
        }
    } else if egui::DragAndDrop::has_payload_of_type::<MorceauGlisse>(ctx) {
        "🎵 Lâche-le ailleurs dans la liste, ou sur une playlist".to_string()
    } else {
        return;
    };
    let Some(souris) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
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
    /// Remplacer la liste de lecture par ces morceaux (clic sur une
    /// playlist), et lancer le morceau numéro `depart` s'il est donné (clic
    /// sur l'un de ses titres) ; sinon, la liste choisit comme d'habitude.
    Charger {
        fichiers: Vec<PathBuf>,
        depart: Option<usize>,
    },
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
    /// On a répondu « Oui » : elle sera supprimée une fois toutes les lignes
    /// dessinées (la supprimer pendant qu'on les parcourt ferait planter).
    suppression_confirmee: bool,
    /// Le champ de saisie vient d'apparaître : on y met le curseur.
    donner_la_main: bool,
    /// Un message affiché sous les playlists (erreur ou confirmation).
    message: String,
    /// La playlist automatique « Les plus écoutés » (voir ecoutes.rs).
    plus_ecoutes: Vec<PathBuf>,
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
            suppression_confirmee: false,
            donner_la_main: false,
            message: String::new(),
            plus_ecoutes: Vec::new(),
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

    /// Remplit la playlist automatique « Les plus écoutés ».
    pub fn changer_les_plus_ecoutes(&mut self, morceaux: Vec<PathBuf>) {
        self.plus_ecoutes = morceaux;
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
        // Un même morceau ne peut être qu'une fois dans une playlist.
        let resultat = lire(&dossier, nom).and_then(|mut contenu| {
            let nouveaux = sans_doublons(&contenu, morceaux);
            let ajoutes = nouveaux.len();
            if ajoutes > 0 {
                contenu.extend(nouveaux);
                ecrire(&dossier, nom, &contenu)?;
            }
            Ok(ajoutes)
        });
        self.message = match resultat {
            Ok(ajoutes) => {
                let deja = morceaux.len() - ajoutes;
                let mut texte = match ajoutes {
                    0 => format!("Déjà dans « {nom} » : rien d'ajouté."),
                    1 => format!("1 morceau ajouté à « {nom} »."),
                    n => format!("{n} morceaux ajoutés à « {nom} »."),
                };
                if ajoutes > 0 && deja > 0 {
                    texte.push_str(&format!(
                        " ({deja} déjà présent{}.)",
                        if deja > 1 { "s" } else { "" }
                    ));
                }
                texte
            }
            Err(e) => format!("Impossible de modifier « {nom} » : {e}"),
        };
        self.actualiser();
    }

    /// Retire le morceau numéro `n` d'une playlist.
    fn retirer_de(&mut self, nom: &str, n: usize) {
        let Some(dossier) = self.dossier.clone() else {
            return;
        };
        let resultat = lire(&dossier, nom).and_then(|mut contenu| {
            if n < contenu.len() {
                let morceau = contenu.remove(n);
                ecrire(&dossier, nom, &contenu)?;
                Ok(Some(morceau))
            } else {
                Ok(None)
            }
        });
        self.message = match resultat {
            Ok(Some(morceau)) => format!("« {} » retiré de « {nom} ».", nom_du_fichier(&morceau)),
            Ok(None) => String::new(),
            Err(e) => format!("Impossible de modifier « {nom} » : {e}"),
        };
        self.actualiser();
    }

    /// Dessine les playlists. `tags` sert à nommer les morceaux d'une
    /// playlist dépliée, comme dans la bibliothèque (« Artiste – Titre »).
    pub fn afficher(&mut self, ui: &mut egui::Ui, tags: &TagsConnus) -> Option<Demande> {
        let mut demande = None;

        theme::separateur(ui, theme::MAGENTA);
        theme::entete_section(ui, "Playlists", theme::MAGENTA, |ui| {
            let nouvelle = theme::Bouton::texte("Nouvelle")
                .avec_icone(theme::Icone::Plus)
                .couleur(theme::MAGENTA)
                .petit()
                .taille(egui::vec2(104.0, 24.0));
            if ui
                .add(nouvelle)
                .on_hover_text("Créer une playlist vide, à remplir en y glissant des morceaux")
                .clicked()
            {
                self.demander_un_nom();
            }
        });
        ui.add_space(4.0);

        if matches!(self.saisie, Some(Saisie::Nouvelle(_))) {
            self.champ_de_saisie(ui);
        }

        if self.dossier.is_none() {
            ui.label("Impossible de trouver où enregistrer les playlists.");
        } else if self.toutes.is_empty() && self.saisie.is_none() {
            ui.label(
                egui::RichText::new("Aucune playlist pour l'instant. Crée-en une avec « + Nouvelle », puis glisses-y des morceaux depuis la bibliothèque.")
                    .color(theme::TEXTE_DISCRET),
            );
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height((ui.available_height() - 24.0).max(40.0))
            .show(ui, |ui| {
                for n in 0..self.toutes.len() {
                    if let Some(d) = self.ligne(ui, n, tags) {
                        demande = Some(d);
                    }
                }
                // Tout en bas, la playlist qui se remplit seule.
                ui.add_space(6.0);
                if let Some(d) = self.ligne_plus_ecoutes(ui, tags) {
                    demande = Some(d);
                }
            });

        if std::mem::take(&mut self.suppression_confirmee) {
            if let Some(nom) = self.a_supprimer.clone() {
                self.supprimer(&nom);
            }
        }

        if !self.message.is_empty() {
            ui.label(egui::RichText::new(&self.message).small().weak());
        }
        demande
    }

    /// Une ligne de playlist : son nom (un clic la charge), et ses boutons.
    fn ligne(&mut self, ui: &mut egui::Ui, n: usize, tags: &TagsConnus) -> Option<Demande> {
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
                    self.suppression_confirmee = true;
                }
                if ui.small_button("Non").clicked() {
                    self.a_supprimer = None;
                }
            });
            return None;
        }

        let mut demande = None;
        let mut a_retirer = None;
        let mut a_lancer = None;
        let mut a_deplacer = None;
        // Comme un dossier de la bibliothèque : la flèche déplie la playlist
        // pour voir ses morceaux (fermée au départ).
        let id = ui.make_persistent_id(("playlist", &nom));
        let chargee = self.chargee.as_deref() == Some(nom.as_str());
        // Le cadre de la ligne (angle coupé en haut à droite) est dessiné
        // sous elle, une fois sa place connue.
        let fond = ui.painter().add(egui::Shape::Noop);
        let entete =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    let texte = egui::RichText::new(&nom).color(if chargee {
                        egui::Color32::WHITE
                    } else {
                        theme::TEXTE
                    });
                    let nombre = self.toutes[n].morceaux.len();
                    let etiquette = ui.add(
                        egui::Label::new(texte)
                            .truncate()
                            .selectable(false)
                            .sense(egui::Sense::click()),
                    );
                    if clic_simple(ui, &etiquette) {
                        if let Some(fichiers) = self.lire_pour(&nom) {
                            self.chargee = Some(nom.clone());
                            demande = Some(Demande::Charger {
                                fichiers,
                                depart: None,
                            });
                        }
                    }
                    // Les boutons, calés à droite.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let petit = |icone| {
                            theme::Bouton::icone(icone)
                                .allure(theme::Allure::Eteint)
                                .couleur(theme::MAGENTA)
                                .taille(egui::vec2(20.0, 20.0))
                        };
                        let supprimer = petit(theme::Icone::Corbeille);
                        if ui.add(supprimer).on_hover_text("Supprimer").clicked() {
                            self.a_supprimer = Some(nom.clone());
                            self.saisie = None;
                        }
                        let renommer = petit(theme::Icone::Crayon);
                        if ui.add(renommer).on_hover_text("Renommer").clicked() {
                            self.saisie = Some(Saisie::Renommer {
                                ancien: nom.clone(),
                                texte: nom.clone(),
                            });
                            self.donner_la_main = true;
                            self.a_supprimer = None;
                        }
                        let bouton = ui
                            .add(petit(theme::Icone::Plus))
                            .on_hover_text("Ajouter à la suite de la liste de lecture");
                        if clic_simple(ui, &bouton) {
                            demande = self.lire_pour(&nom).map(Demande::AjouterALaListe);
                        }
                        ui.label(
                            crate::police::chiffres(nombre.to_string())
                                .size(11.0)
                                .color(theme::TEXTE_DISCRET),
                        );
                    });
                });
        let rangee = entete
            .body(|ui| {
                let morceaux = &self.toutes[n].morceaux;
                if morceaux.is_empty() {
                    ui.weak("Vide : glisse des morceaux de la bibliothèque sur la playlist.");
                }
                // La place de chaque ligne, pour savoir où lâcher un titre glissé.
                let mut lignes = Vec::with_capacity(morceaux.len());
                for (i, morceau) in morceaux.iter().enumerate() {
                    let survol = ui.painter().add(egui::Shape::Noop);
                    let ligne = ui.horizontal(|ui| {
                        // Un clic lance ce titre ; le faire glisser le déplace.
                        let titre = ui.add(
                            egui::Label::new(tags.libelle(morceau))
                                .selectable(false)
                                .sense(egui::Sense::click_and_drag()),
                        );
                        if clic_simple(ui, &titre) {
                            a_lancer = Some(i);
                        }
                        if titre.drag_started() {
                            egui::DragAndDrop::set_payload(
                                ui.ctx(),
                                TitreGlisse {
                                    playlist: nom.clone(),
                                    depuis: i,
                                },
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("✖")
                                .on_hover_text("Retirer de la playlist")
                                .clicked()
                            {
                                a_retirer = Some(i);
                            }
                        });
                    });
                    theme::fond_de_survol(ui, survol, ligne.response.rect);
                    lignes.push(ligne.response.rect);
                }
                a_deplacer = deposer_un_titre(ui, &nom, &lignes);
            })
            .1;
        let cadre = rangee.response.rect.expand2(egui::vec2(4.0, 3.0));
        let coins = theme::Coins::haut_droit(10.0);
        // Au survol, le même fond cyan léger que les lignes de la liste de lecture.
        let survol = ui.rect_contains_pointer(rangee.response.rect);
        let remplissage = if chargee {
            theme::attenue(theme::MAGENTA, 40)
        } else if survol {
            theme::couleur_de_survol()
        } else {
            theme::attenue(theme::PANNEAU, 200)
        };
        let forme = egui::Shape::convex_polygon(
            theme::contour(cadre, coins),
            remplissage,
            egui::Stroke::new(
                1.0_f32,
                theme::attenue(theme::MAGENTA, if chargee { 160 } else { 68 }),
            ),
        );
        ui.painter().set(fond, forme);
        if let Some(i) = a_retirer {
            self.retirer_de(&nom, i);
        }
        if let Some((depuis, vers)) = a_deplacer {
            self.deplacer_dans(&nom, depuis, vers);
        }
        if let Some(i) = a_lancer {
            demande = self.lancer_depuis(n, i);
        }

        // Des morceaux glissés depuis la bibliothèque ou la liste de lecture
        // survolent la ligne : on l'encadre, et s'ils sont lâchés ici, on les
        // ajoute à la playlist.
        let ctx = ui.ctx().clone();
        if (egui::DragAndDrop::has_payload_of_type::<MorceauxGlisses>(&ctx)
            || egui::DragAndDrop::has_payload_of_type::<MorceauGlisse>(&ctx))
            && ui.rect_contains_pointer(rangee.response.rect)
        {
            ui.painter().rect_stroke(
                rangee.response.rect.expand(2.0),
                3.0,
                ui.visuals().selection.stroke,
            );
            if ctx.input(|i| i.pointer.any_released()) {
                // Attention : `take_payload` vide le glissement même s'il n'est
                // pas du type demandé. On regarde donc d'abord ce qu'il porte.
                if let Some(glisses) = egui::DragAndDrop::payload::<MorceauxGlisses>(&ctx) {
                    self.ajouter_a(&nom, &glisses.0);
                } else if let Some(morceau) = egui::DragAndDrop::payload::<MorceauGlisse>(&ctx) {
                    self.ajouter_a(&nom, std::slice::from_ref(&morceau.fichier));
                }
                egui::DragAndDrop::clear_payload(&ctx);
            }
        }
        demande
    }

    /// La ligne de « Les plus écoutés » : comme une playlist, mais en cyan,
    /// sans renommer, supprimer ni déposer de morceaux (elle se remplit seule).
    fn ligne_plus_ecoutes(&mut self, ui: &mut egui::Ui, tags: &TagsConnus) -> Option<Demande> {
        let mut demande = None;
        let mut a_lancer = None;
        let id = ui.make_persistent_id("playlist automatique : plus écoutés");
        let chargee = self.chargee.as_deref() == Some(PLUS_ECOUTES);
        let fond = ui.painter().add(egui::Shape::Noop);
        let entete =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    let texte = egui::RichText::new("Les plus écoutés").color(if chargee {
                        egui::Color32::WHITE
                    } else {
                        theme::CYAN
                    });
                    let etiquette = ui
                        .add(
                            egui::Label::new(texte)
                                .truncate()
                                .selectable(false)
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text(
                            "Se remplit toute seule avec tes morceaux les plus écoutés \
                             (les écoutes récentes comptent plus).",
                        );
                    if clic_simple(ui, &etiquette) && !self.plus_ecoutes.is_empty() {
                        self.chargee = Some(PLUS_ECOUTES.to_string());
                        demande = Some(Demande::Charger {
                            fichiers: self.plus_ecoutes.clone(),
                            depart: None,
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ajouter = theme::Bouton::icone(theme::Icone::Plus)
                            .allure(theme::Allure::Eteint)
                            .couleur(theme::CYAN)
                            .taille(egui::vec2(20.0, 20.0));
                        let bouton = ui
                            .add(ajouter)
                            .on_hover_text("Ajouter à la suite de la liste de lecture");
                        if clic_simple(ui, &bouton) && !self.plus_ecoutes.is_empty() {
                            demande = Some(Demande::AjouterALaListe(self.plus_ecoutes.clone()));
                        }
                        ui.label(
                            crate::police::chiffres(self.plus_ecoutes.len().to_string())
                                .size(11.0)
                                .color(theme::TEXTE_DISCRET),
                        );
                    });
                });
        let rangee = entete
            .body(|ui| {
                if self.plus_ecoutes.is_empty() {
                    ui.weak("Vide pour l'instant : un morceau y entre quand tu en as écouté au moins la moitié.");
                }
                for (i, morceau) in self.plus_ecoutes.iter().enumerate() {
                    let survol = ui.painter().add(egui::Shape::Noop);
                    let titre = ui.add(
                        egui::Label::new(tags.libelle(morceau))
                            .selectable(false)
                            .sense(egui::Sense::click()),
                    );
                    let ligne = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), titre.rect.y_range());
                    theme::fond_de_survol(ui, survol, ligne);
                    if clic_simple(ui, &titre) {
                        a_lancer = Some(i);
                    }
                }
            })
            .1;
        let cadre = rangee.response.rect.expand2(egui::vec2(4.0, 3.0));
        // Au survol, le même fond cyan léger que les lignes de la liste de lecture.
        let survol = ui.rect_contains_pointer(rangee.response.rect);
        let remplissage = if chargee {
            theme::attenue(theme::CYAN, 40)
        } else if survol {
            theme::couleur_de_survol()
        } else {
            theme::attenue(theme::PANNEAU, 200)
        };
        let forme = egui::Shape::convex_polygon(
            theme::contour(cadre, theme::Coins::haut_droit(10.0)),
            remplissage,
            egui::Stroke::new(
                1.0_f32,
                theme::attenue(theme::CYAN, if chargee { 160 } else { 68 }),
            ),
        );
        ui.painter().set(fond, forme);
        if let Some(i) = a_lancer {
            self.chargee = Some(PLUS_ECOUTES.to_string());
            demande = Some(Demande::Charger {
                fichiers: self.plus_ecoutes.clone(),
                depart: Some(i),
            });
        }
        demande
    }

    /// Clic sur le titre numéro `i` de la playlist numéro `n` : la playlist
    /// remplace la liste de lecture, et ce titre est lancé.
    fn lancer_depuis(&mut self, n: usize, i: usize) -> Option<Demande> {
        let nom = self.toutes[n].nom.clone();
        let titre = self.toutes[n].morceaux.get(i)?.clone();
        let fichiers = self.lire_pour(&nom)?;
        // Les morceaux introuvables ont été écartés : on retrouve le titre.
        let Some(depart) = fichiers.iter().position(|f| *f == titre) else {
            self.message = format!("Ce fichier est introuvable : {}", titre.display());
            return None;
        };
        self.chargee = Some(nom);
        Some(Demande::Charger {
            fichiers,
            depart: Some(depart),
        })
    }

    /// Déplace un titre dans une playlist (glisser-déposer). `vers` est la
    /// place où on l'a lâché, comptée avant de l'enlever de la sienne.
    fn deplacer_dans(&mut self, nom: &str, depuis: usize, vers: usize) {
        let Some(dossier) = self.dossier.clone() else {
            return;
        };
        let resultat = lire(&dossier, nom).and_then(|mut contenu| {
            if deplacer(&mut contenu, depuis, vers) {
                ecrire(&dossier, nom, &contenu)?;
            }
            Ok(())
        });
        if let Err(e) = resultat {
            self.message = format!("Impossible de modifier « {nom} » : {e}");
        }
        self.actualiser();
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

/// Pendant qu'on fait glisser un titre de cette playlist sur ses lignes
/// (`lignes` : leur place à l'écran), on dessine un trait là où il irait.
/// S'il est lâché, renvoie (sa place d'origine, sa nouvelle place).
fn deposer_un_titre(ui: &egui::Ui, nom: &str, lignes: &[egui::Rect]) -> Option<(usize, usize)> {
    let ctx = ui.ctx();
    let glisse = egui::DragAndDrop::payload::<TitreGlisse>(ctx)?;
    if glisse.playlist != nom {
        return None;
    }
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let zone = lignes.iter().copied().reduce(|a, b| a.union(b))?;
    let souris = ctx.input(|i| i.pointer.hover_pos())?;
    if !zone.expand2(egui::vec2(0.0, 6.0)).contains(souris) {
        return None;
    }
    // Comme dans la liste de lecture : avant la première ligne dont le
    // milieu est sous la souris.
    let vers = place_sous_la_souris(lignes, souris.y);
    let y = match lignes.get(vers) {
        Some(r) => r.top() - ui.spacing().item_spacing.y / 2.0,
        None => zone.bottom() + ui.spacing().item_spacing.y / 2.0,
    };
    let trait_ = egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color);
    ui.painter().hline(zone.x_range(), y, trait_);
    if ctx.input(|i| i.pointer.any_released()) {
        egui::DragAndDrop::clear_payload(ctx);
        return Some((glisse.depuis, vers));
    }
    None
}

/// Déplace l'élément `depuis` pour qu'il arrive à la place `vers` (comptée
/// avant de l'enlever). Renvoie `false` si rien ne bouge.
fn deplacer<T>(liste: &mut Vec<T>, depuis: usize, vers: usize) -> bool {
    if depuis >= liste.len() || vers > liste.len() || vers == depuis || vers == depuis + 1 {
        return false;
    }
    let element = liste.remove(depuis);
    let vers = if vers > depuis { vers - 1 } else { vers };
    liste.insert(vers, element);
    true
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
    if nom.contains(INTERDITS) || nom.contains(char::is_control) || nom.ends_with('.') {
        return Err(
            "Un nom de playlist ne peut pas contenir \\ / : * ? \" < > | ni finir par un point."
                .to_string(),
        );
    }
    if est_reserve_par_windows(nom) {
        return Err(format!(
            "« {nom} » est un nom réservé par Windows, choisis-en un autre."
        ));
    }
    Ok(nom.to_string())
}

/// Windows réserve ces noms à ses appareils (CON = l'écran, NUL = nulle
/// part…), même suivis d'une extension : « nul.m3u » n'enregistrerait rien.
fn est_reserve_par_windows(nom: &str) -> bool {
    let base = nom
        .split('.')
        .next()
        .unwrap_or(nom)
        .trim_end()
        .to_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0')
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
            let morceaux = lire(dossier, &nom).unwrap_or_default();
            Some(Playlist { nom, morceaux })
        })
        .collect();
    toutes.sort_by_cached_key(|p| p.nom.to_lowercase());
    toutes
}

/// Parmi `nouveaux`, ceux qui ne sont pas déjà dans `existants` (ni deux
/// fois dans `nouveaux`). Comme pour Windows, les majuscules ne comptent pas.
fn sans_doublons(existants: &[PathBuf], nouveaux: &[PathBuf]) -> Vec<PathBuf> {
    let cle = |chemin: &PathBuf| chemin.to_string_lossy().to_lowercase();
    let mut vus: std::collections::HashSet<String> = existants.iter().map(cle).collect();
    nouveaux
        .iter()
        .filter(|chemin| vus.insert(cle(chemin)))
        .cloned()
        .collect()
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
    let texte = texte_du_m3u(std::fs::read(chemin_de(dossier, nom))?);
    Ok(lignes_de_morceaux(&texte)
        .map(|ligne| dossier.join(ligne))
        .collect())
}

/// Le texte d'un .m3u. Nous l'écrivons en UTF-8, mais Winamp ou le Lecteur
/// Windows Media l'écrivent souvent en « ANSI » (Windows-1252) : si ce
/// n'est pas de l'UTF-8 valide, on le lit comme tel, accents compris.
fn texte_du_m3u(octets: Vec<u8>) -> String {
    String::from_utf8(octets)
        .unwrap_or_else(|e| e.into_bytes().iter().map(|&o| windows_1252(o)).collect())
}

/// Le caractère d'un octet en Windows-1252. C'est le Latin-1 (é = 0xE9…),
/// sauf de 0x80 à 0x9F, où Windows a placé €, les guillemets, œ, etc.
fn windows_1252(octet: u8) -> char {
    const DE_80_A_9F: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match octet {
        0x80..=0x9f => DE_80_A_9F[usize::from(octet - 0x80)],
        _ => char::from(octet),
    }
}

/// Écrit (ou réécrit) une playlist.
fn ecrire(dossier: &Path, nom: &str, morceaux: &[PathBuf]) -> io::Result<()> {
    std::fs::create_dir_all(dossier)?;
    let mut texte = String::from("#EXTM3U\r\n");
    for morceau in morceaux {
        texte.push_str(&morceau.to_string_lossy());
        texte.push_str("\r\n");
    }
    ecrire_sans_risque(&chemin_de(dossier, nom), texte.as_bytes())
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
        let racine = std::env::temp_dir().join(format!("dollwave_test_playlists_{nom}"));
        let _ = std::fs::remove_dir_all(&racine);
        racine
    }

    fn noms(dossier: &Path) -> Vec<(String, usize)> {
        lister(dossier)
            .into_iter()
            .map(|p| (p.nom, p.morceaux.len()))
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

    /// Un chemin absolu sur toutes les machines : « /m/1.mp3 » n'en est
    /// pas un pour Windows, qui le rattache au disque C:.
    fn absolu(chemin: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{chemin}"))
        } else {
            PathBuf::from(chemin)
        }
    }

    #[test]
    fn ajoute_a_la_fin() {
        let dossier = dossier_de_test("ajoute");
        creer(&dossier, "p", &[absolu("/m/1.mp3")]).unwrap();
        let mut playlists = Playlists::dans(Some(dossier.clone()));
        playlists.ajouter_a("p", &[absolu("/m/2.mp3"), absolu("/m/3.mp3")]);
        assert_eq!(
            lire(&dossier, "p").unwrap(),
            vec![absolu("/m/1.mp3"), absolu("/m/2.mp3"), absolu("/m/3.mp3")]
        );
        assert_eq!(playlists.toutes[0].morceaux.len(), 3);
        // Un morceau déjà présent n'est pas ajouté une deuxième fois, même
        // glissé deux fois de suite.
        playlists.ajouter_a(
            "p",
            &[absolu("/m/2.mp3"), absolu("/m/4.mp3"), absolu("/m/4.mp3")],
        );
        assert_eq!(playlists.toutes[0].morceaux.len(), 4);
        playlists.ajouter_a("p", &[absolu("/M/1.MP3")]);
        assert_eq!(playlists.toutes[0].morceaux.len(), 4);
        // Retirer un morceau.
        playlists.retirer_de("p", 0);
        assert_eq!(
            lire(&dossier, "p").unwrap(),
            vec![absolu("/m/2.mp3"), absolu("/m/3.mp3"), absolu("/m/4.mp3")]
        );
    }

    #[test]
    fn deplace_un_titre() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        // « a » lâché avant « d » (place 3).
        assert!(deplacer(&mut v, 0, 3));
        assert_eq!(v, ['b', 'c', 'a', 'd']);
        // « d » lâché tout en haut.
        assert!(deplacer(&mut v, 3, 0));
        assert_eq!(v, ['d', 'b', 'c', 'a']);
        // « b » lâché tout en bas.
        assert!(deplacer(&mut v, 1, 4));
        assert_eq!(v, ['d', 'c', 'a', 'b']);
        // Lâché juste avant ou juste après lui-même : rien ne bouge.
        assert!(!deplacer(&mut v, 2, 2));
        assert!(!deplacer(&mut v, 2, 3));
        assert_eq!(v, ['d', 'c', 'a', 'b']);
    }

    #[test]
    fn lit_un_m3u_d_un_autre_logiciel() {
        let dossier = dossier_de_test("autre");
        std::fs::create_dir_all(&dossier).unwrap();
        std::fs::write(
            dossier.join("vlc.m3u"),
            format!(
                "\u{feff}#EXTM3U\n#EXTINF:123,Artiste - Titre\nsous/dossier/1.mp3\n\n{}\n",
                absolu("/absolu/2.mp3").display()
            ),
        )
        .unwrap();
        assert_eq!(
            lire(&dossier, "vlc").unwrap(),
            vec![dossier.join("sous/dossier/1.mp3"), absolu("/absolu/2.mp3")]
        );
    }

    /// La hauteur du milieu de la première playlist à l'écran (sous le
    /// séparateur, l'en-tête « PLAYLISTS » et sa marge).
    const PREMIERE_LIGNE: f32 = 61.0;

    /// Glisse un morceau de la liste de lecture (à droite) jusqu'à la
    /// hauteur `y` du panneau des playlists (à gauche), et lâche.
    fn glisser_de_la_liste_vers(y: f32, dossier: &Path) {
        use crate::liste::ListeDeLecture;
        let ctx = egui::Context::default();
        let tags = TagsConnus::new(&ctx);
        let mut playlists = Playlists::dans(Some(dossier.to_path_buf()));
        let mut liste = ListeDeLecture::default();
        liste.ajouter((0..3).map(|i| PathBuf::from(format!("{i}.mp3"))).collect());
        let mut t = 0.0;
        let mut image = |events: Vec<egui::Event>| {
            t += 0.02;
            let entree = egui::RawInput {
                time: Some(t),
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 400.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(entree, |ctx| {
                egui::SidePanel::left("gauche")
                    .exact_width(300.0)
                    .show(ctx, |ui| {
                        playlists.afficher(ui, &tags);
                    });
                egui::CentralPanel::default().show(ctx, |ui| {
                    liste.afficher(ui, &tags, &[]);
                });
                dessiner_glissement(ctx);
            });
        };
        let bouton = |pos: egui::Pos2, pressed: bool| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        // Le 1er morceau de la liste, et la 1re playlist (vers PREMIERE_LIGNE).
        let depart = egui::pos2(380.0, 15.0);
        let arrivee = egui::pos2(60.0, y);
        image(vec![]);
        image(vec![]);
        image(bouton(depart, true));
        for i in 1..=20 {
            let p = depart + (arrivee - depart) * (i as f32 / 20.0);
            image(vec![egui::Event::PointerMoved(p)]);
        }
        image(bouton(arrivee, false));
        image(vec![]);
    }

    #[test]
    fn glisser_un_morceau_de_la_liste_sur_une_playlist() {
        let dossier = dossier_de_test("depuis_la_liste");
        creer(&dossier, "Soirée", &[]).unwrap();
        // Lâché à côté de la playlist : rien.
        glisser_de_la_liste_vers(150.0, &dossier);
        assert!(lire(&dossier, "Soirée").unwrap().is_empty());
        // Lâché dessus : le morceau y est ajouté.
        glisser_de_la_liste_vers(PREMIERE_LIGNE, &dossier);
        assert_eq!(lire(&dossier, "Soirée").unwrap(), [dossier.join("0.mp3")]);
        std::fs::remove_dir_all(&dossier).unwrap();
    }

    /// Bug de l'audit : répondre « Oui » pour supprimer une playlist qui
    /// n'est pas la dernière faisait planter l'appli.
    #[test]
    fn supprimer_la_premiere_playlist_depuis_l_ecran() {
        let dossier = dossier_de_test("supprimer_ecran");
        creer(&dossier, "a", &[]).unwrap();
        creer(&dossier, "b", &[]).unwrap();
        let ctx = egui::Context::default();
        let tags = TagsConnus::new(&ctx);
        let mut playlists = Playlists::dans(Some(dossier.clone()));
        // On a cliqué sur 🗑 de « a » : la ligne demande confirmation.
        playlists.a_supprimer = Some("a".to_string());
        let mut t = 0.0;
        let mut image = |playlists: &mut Playlists, events: Vec<egui::Event>| {
            t += 0.02;
            let entree = egui::RawInput {
                time: Some(t),
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 400.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(entree, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    playlists.afficher(ui, &tags);
                });
            });
        };
        let clic = |pos: egui::Pos2, pressed: bool| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        image(&mut playlists, vec![]);
        image(&mut playlists, vec![]);
        // On ne connaît pas la place exacte du bouton « Oui » : on clique de
        // gauche à droite sur la 1re ligne. Le texte de la question ne réagit
        // pas, et « Oui » vient avant « Non ».
        let mut x = 5.0;
        while playlists.noms().len() == 2 && x < 300.0 {
            let pos = egui::pos2(x, PREMIERE_LIGNE);
            image(&mut playlists, clic(pos, true));
            image(&mut playlists, clic(pos, false));
            image(&mut playlists, vec![]);
            x += 4.0;
        }
        assert_eq!(playlists.noms(), vec!["b".to_string()]);
        assert!(playlists.a_supprimer.is_none());
        std::fs::remove_dir_all(&dossier).unwrap();
    }

    #[test]
    fn lit_un_m3u_ecrit_en_ansi_par_windows() {
        let dossier = dossier_de_test("ansi");
        std::fs::create_dir_all(&dossier).unwrap();
        // « Café – 5 €.mp3 » écrit en Windows-1252 par Winamp.
        let mut octets = b"#EXTM3U\r\nC:\\Musique\\Caf\xe9 \x96 5 \x80.mp3\r\n".to_vec();
        octets.extend_from_slice(b"C:\\Musique\\\x9cuvre.mp3\r\n");
        std::fs::write(chemin_de(&dossier, "Winamp"), octets).unwrap();
        let morceaux = lire(&dossier, "Winamp").unwrap();
        let noms: Vec<String> = morceaux
            .iter()
            .map(|m| m.to_string_lossy().rsplit('\\').next().unwrap().to_string())
            .collect();
        assert_eq!(noms, ["Café – 5 €.mp3", "œuvre.mp3"]);
        // Et un .m3u en UTF-8 (le nôtre) se lit toujours pareil.
        creer(&dossier, "Nôtre", &[PathBuf::from("Été.mp3")]).unwrap();
        assert_eq!(lire(&dossier, "Nôtre").unwrap(), [dossier.join("Été.mp3")]);
        std::fs::remove_dir_all(&dossier).unwrap();
    }

    #[test]
    fn refuse_les_noms_reserves_par_windows() {
        for nom in ["CON", "nul", "Aux", "prn.txt", "COM1", "lpt9", "NUL "] {
            assert!(nom_valide(nom).is_err(), "{nom} devrait être refusé");
        }
        for nom in [
            "Console",
            "COM",
            "COM10",
            "LPT0",
            "Nul part",
            "Auxerre",
            "Con Funk Shun",
        ] {
            assert!(nom_valide(nom).is_ok(), "{nom} devrait être accepté");
        }
        assert!(nom_valide("A\tB").is_err());
    }

    #[test]
    fn ecrire_remplace_sans_laisser_de_copie() {
        let dossier = dossier_de_test("sans_risque");
        creer(&dossier, "Soirée", &[PathBuf::from("a.mp3")]).unwrap();
        ecrire(
            &dossier,
            "Soirée",
            &[PathBuf::from("b.mp3"), PathBuf::from("c.mp3")],
        )
        .unwrap();
        assert_eq!(lire(&dossier, "Soirée").unwrap().len(), 2);
        let fichiers: Vec<_> = std::fs::read_dir(&dossier)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(fichiers, ["Soirée.m3u"]);
        std::fs::remove_dir_all(&dossier).unwrap();
    }
}
