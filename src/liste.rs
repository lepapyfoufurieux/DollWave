//! La liste de lecture : les morceaux à jouer les uns après les autres,
//! et lequel est en cours.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::format_temps;
use crate::playlists::menu_ajouter_a_une_playlist;
use crate::tags::TagsConnus;

/// Hauteur maximale de la liste à l'écran ; au-delà, on fait défiler.
const HAUTEUR_MAX: f32 = 160.0;

/// Ce que l'utilisateur a demandé en cliquant dans la liste.
pub enum Action {
    /// Jouer le morceau numéro n (clic sur sa ligne).
    Jouer(usize),
    /// Retirer le morceau numéro n de la liste (bouton ✖).
    Retirer(usize),
    /// Ajouter ce morceau à la playlist `nom` (clic droit).
    AjouterAPlaylist { nom: String, fichier: PathBuf },
}

#[derive(Default)]
pub struct ListeDeLecture {
    morceaux: Vec<PathBuf>,
    /// Le numéro du morceau en cours dans la liste, s'il y en a un.
    courant: Option<usize>,
    /// Lecture aléatoire activée ?
    aleatoire: bool,
    /// En lecture aléatoire : pour chaque morceau, a-t-il déjà été joué ?
    /// Chaque morceau passe une fois avant que la lecture s'arrête.
    deja_joue: Vec<bool>,
    /// Répétition activée ? Au bout de la liste, on repart pour un tour.
    repetition: bool,
    /// En lecture aléatoire : les morceaux joués, dans l'ordre, pour que
    /// « précédent » revienne vraiment au morceau d'avant.
    historique: Vec<usize>,
    /// Le morceau en cours lors du dernier affichage. S'il a changé depuis,
    /// on fait défiler la liste jusqu'au nouveau.
    courant_affiche: Option<usize>,
    /// En lecture aléatoire : le morceau demandé avec « Lire ensuite », qui
    /// passe avant le hasard.
    prochain: Option<usize>,
    /// La hauteur d'une ligne à l'écran, pour réserver la place de celles
    /// qu'on ne dessine pas (hors de l'écran).
    hauteur_ligne: Option<f32>,
}

/// Ce qu'on transporte quand on glisse un morceau de la liste : son numéro,
/// pour le lâcher ailleurs dans la liste, et son fichier, pour le lâcher
/// sur une playlist.
pub struct MorceauGlisse {
    n: usize,
    pub fichier: PathBuf,
}

impl ListeDeLecture {
    /// Ajoute des fichiers à la fin de la liste. Renvoie le numéro du morceau
    /// à lancer si rien ne joue : le premier ajouté, ou en lecture aléatoire
    /// l'un des ajoutés au hasard. Rien si la liste n'a pas grandi.
    pub fn ajouter(&mut self, fichiers: Vec<PathBuf>) -> Option<usize> {
        let premier = self.morceaux.len();
        self.morceaux.extend(fichiers);
        self.deja_joue.resize(self.morceaux.len(), false);
        let ajoutes = self.morceaux.len() - premier;
        if ajoutes == 0 {
            None
        } else if self.aleatoire {
            Some(premier + hasard(ajoutes))
        } else {
            Some(premier)
        }
    }

    /// Retire un morceau. Le numéro du morceau en cours est décalé si besoin.
    /// Renvoie `true` si c'est le morceau en cours qui a été retiré.
    pub fn retirer(&mut self, n: usize) -> bool {
        if n >= self.morceaux.len() {
            return false;
        }
        self.morceaux.remove(n);
        self.deja_joue.remove(n);
        self.prochain = match self.prochain {
            Some(p) if p == n => None,
            Some(p) if p > n => Some(p - 1),
            p => p,
        };
        // Dans l'historique, on oublie ce morceau et on décale les suivants.
        self.historique.retain(|&h| h != n);
        for h in &mut self.historique {
            if *h > n {
                *h -= 1;
            }
        }
        match self.courant {
            Some(c) if c == n => {
                self.courant = None;
                true
            }
            Some(c) if c > n => {
                self.courant = Some(c - 1);
                false
            }
            _ => false,
        }
    }

    pub fn vider(&mut self) {
        self.morceaux.clear();
        self.deja_joue.clear();
        self.historique.clear();
        self.courant = None;
        self.prochain = None;
    }

    pub fn aleatoire(&self) -> bool {
        self.aleatoire
    }

    /// Active ou désactive la lecture aléatoire. En l'activant, on repart
    /// pour un nouveau tour : seul le morceau en cours compte comme déjà joué.
    pub fn changer_aleatoire(&mut self, actif: bool) {
        self.aleatoire = actif;
        self.prochain = None;
        self.deja_joue.fill(false);
        self.historique.clear();
        if let Some(c) = self.courant {
            self.deja_joue[c] = true;
            self.historique.push(c);
        }
    }

    pub fn repetition(&self) -> bool {
        self.repetition
    }

    pub fn changer_repetition(&mut self, actif: bool) {
        self.repetition = actif;
    }

    /// Tous les morceaux de la liste, dans l'ordre.
    pub fn morceaux(&self) -> &[PathBuf] {
        &self.morceaux
    }

    pub fn est_vide(&self) -> bool {
        self.morceaux.is_empty()
    }

    /// Le fichier du morceau numéro n.
    pub fn fichier(&self, n: usize) -> Option<&Path> {
        self.morceaux.get(n).map(PathBuf::as_path)
    }

    pub fn courant(&self) -> Option<usize> {
        self.courant
    }

    pub fn choisir(&mut self, n: usize) {
        self.courant = Some(n);
        if self.prochain == Some(n) {
            self.prochain = None;
        }
        if let Some(joue) = self.deja_joue.get_mut(n) {
            *joue = true;
        }
        if self.historique.last() != Some(&n) {
            self.historique.push(n);
        }
    }

    /// Le numéro du morceau suivant, ou rien si on est au bout de la liste
    /// (en aléatoire : quand tous les morceaux ont été joués). Avec la
    /// répétition, au bout de la liste on repart du premier ; en aléatoire,
    /// on remélange pour un nouveau tour.
    pub fn suivant(&mut self) -> Option<usize> {
        if self.morceaux.is_empty() {
            return None;
        }
        if self.aleatoire {
            if let Some(p) = self.prochain.take() {
                return Some(p);
            }
            let mut restants: Vec<usize> = (0..self.morceaux.len())
                .filter(|&i| !self.deja_joue[i])
                .collect();
            if restants.is_empty() {
                if !self.repetition {
                    return None;
                }
                // Nouveau tour : tout le monde peut repasser, sauf le morceau
                // qui vient de jouer (pour ne pas l'entendre deux fois de suite).
                self.deja_joue.fill(false);
                restants = (0..self.morceaux.len())
                    .filter(|&i| self.morceaux.len() == 1 || Some(i) != self.courant)
                    .collect();
            }
            return Some(restants[hasard(restants.len())]);
        }
        let n = self.courant.map_or(0, |c| c + 1);
        if n < self.morceaux.len() {
            Some(n)
        } else {
            self.repetition.then_some(0)
        }
    }

    /// Le numéro du morceau précédent. Au début de la liste, on reste sur le
    /// premier (il sera relancé depuis le début). En aléatoire, on revient au
    /// morceau joué juste avant.
    pub fn precedent(&mut self) -> Option<usize> {
        if self.morceaux.is_empty() {
            return None;
        }
        if self.aleatoire {
            if self.historique.len() >= 2 {
                self.historique.pop(); // on quitte le morceau en cours
            }
            return self.historique.last().copied().or(self.courant).or(Some(0));
        }
        Some(self.courant.map_or(0, |c| c.saturating_sub(1)))
    }

    /// Déplace le morceau numéro `de` pour l'insérer à la place `vers`
    /// (0 : tout en haut ; le nombre de morceaux : tout en bas), comptée dans
    /// la liste d'avant le déplacement. Le morceau en cours, l'historique et
    /// les morceaux déjà joués suivent leur morceau.
    pub fn deplacer(&mut self, de: usize, vers: usize) {
        let taille = self.morceaux.len();
        if de >= taille || vers > taille {
            return;
        }
        // Une fois le morceau retiré, tout ce qui le suivait remonte d'un cran.
        let arrivee = if vers > de { vers - 1 } else { vers };
        if arrivee == de {
            return;
        }
        // Le nouveau numéro d'un morceau qui avait le numéro i.
        let nouveau = |i: usize| {
            if i == de {
                arrivee
            } else if de < i && i <= arrivee {
                i - 1
            } else if arrivee <= i && i < de {
                i + 1
            } else {
                i
            }
        };
        let morceau = self.morceaux.remove(de);
        self.morceaux.insert(arrivee, morceau);
        let joue = self.deja_joue.remove(de);
        self.deja_joue.insert(arrivee, joue);
        for h in &mut self.historique {
            *h = nouveau(*h);
        }
        self.courant = self.courant.map(nouveau);
        self.courant_affiche = self.courant_affiche.map(nouveau);
        self.prochain = self.prochain.map(nouveau);
    }

    /// « Lire ensuite » : place le morceau juste après celui en cours (en tête
    /// de liste si rien ne joue), pour qu'il passe au prochain morceau. En
    /// lecture aléatoire, il passe aussi avant le hasard.
    pub fn lire_ensuite(&mut self, n: usize) {
        if n >= self.morceaux.len() || self.courant == Some(n) {
            return;
        }
        let vers = self.courant.map_or(0, |c| c + 1);
        self.deplacer(n, vers);
        let arrivee = if vers > n { vers - 1 } else { vers };
        if self.aleatoire {
            self.prochain = Some(arrivee);
        }
    }

    /// Dessine la liste. Renvoie ce que l'utilisateur a demandé, s'il a cliqué.
    /// Chaque morceau est nommé d'après ses tags (« Artiste – Titre »), avec
    /// sa durée à droite.
    /// `playlists` : les noms des playlists, pour le menu du clic droit.
    pub fn afficher(
        &mut self,
        ui: &mut egui::Ui,
        tags: &TagsConnus,
        playlists: &[String],
    ) -> Option<Action> {
        let mut action = None;
        let mut a_lire_ensuite = None;
        // Les rectangles des lignes, pour savoir où lâcher un morceau glissé.
        let mut lignes: Vec<egui::Rect> = Vec::with_capacity(self.morceaux.len());
        let glisse = egui::DragAndDrop::payload::<MorceauGlisse>(ui.ctx()).map(|m| m.n);
        // On ne défile que quand le morceau change : entre-temps, on peut
        // faire défiler la liste à la main sans qu'elle revienne toute seule.
        let defiler = self.courant != self.courant_affiche;
        self.courant_affiche = self.courant;
        egui::ScrollArea::vertical()
            .max_height(HAUTEUR_MAX)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let mut hauteur = self.hauteur_ligne;
                for (n, fichier) in self.morceaux.iter().enumerate() {
                    let en_cours = self.courant == Some(n);
                    let (place, _) = rangee(ui, &mut hauteur, n, |ui| {
                        if ui.small_button("✖").on_hover_text("Retirer").clicked() {
                            action = Some(Action::Retirer(n));
                        }
                        if ui
                            .add_enabled(!en_cours, egui::Button::new("⏭").small())
                            .on_hover_text("Lire ensuite")
                            .clicked()
                        {
                            a_lire_ensuite = Some(n);
                        }
                        let texte = format!("{}. {}", n + 1, tags.libelle(fichier));
                        // Le morceau qu'on est en train de glisser est grisé.
                        let texte = if glisse == Some(n) {
                            egui::RichText::new(texte).weak()
                        } else {
                            egui::RichText::new(texte)
                        };
                        // Un clic lance le morceau ; si on bouge la souris en
                        // gardant le bouton appuyé, on le glisse ailleurs.
                        let ligne = ui
                            .selectable_label(en_cours, texte)
                            .interact(egui::Sense::drag());
                        if ligne.drag_started() {
                            egui::DragAndDrop::set_payload(
                                ui.ctx(),
                                MorceauGlisse {
                                    n,
                                    fichier: fichier.clone(),
                                },
                            );
                        }
                        if let Some(nom) = menu_ajouter_a_une_playlist(&ligne, playlists) {
                            action = Some(Action::AjouterAPlaylist {
                                nom,
                                fichier: fichier.clone(),
                            });
                        }
                        // L'infobulle n'est préparée que si la souris est dessus.
                        let ligne = if glisse.is_some() {
                            ligne
                        } else {
                            ligne.on_hover_ui(|ui| {
                                ui.label(tags.infobulle(fichier));
                            })
                        };
                        if let Some(duree) = tags.duree(fichier) {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| ui.weak(format_temps(duree)),
                            );
                        }
                        // Un clic suffit. Pour egui, un double-clic, c'est deux
                        // clics : le second tombe sur le morceau devenu le
                        // morceau en cours, ce qui ne le relance pas (voir main.rs).
                        if ligne.clicked() {
                            action = Some(Action::Jouer(n));
                        }
                    });
                    // Le morceau en cours vient de changer : on fait défiler la
                    // liste jusqu'à lui, même s'il était hors de l'écran.
                    if en_cours && defiler {
                        ui.scroll_to_rect(place, None);
                    }
                    lignes.push(place);
                }
                self.hauteur_ligne = hauteur;
                if let Some(de) = glisse {
                    self.glisser(ui, de, &lignes);
                }
            });
        if let Some(n) = a_lire_ensuite {
            self.lire_ensuite(n);
        }
        action
    }

    /// Pendant qu'on glisse le morceau `de` : montre par un trait où il va
    /// atterrir, et le déplace quand on lâche le bouton de la souris.
    fn glisser(&mut self, ui: &mut egui::Ui, de: usize, lignes: &[egui::Rect]) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let Some(souris) = ui.ctx().pointer_interact_pos() else {
            return;
        };
        let zone = ui.clip_rect();
        // Près du haut ou du bas de la liste, on la fait défiler.
        if souris.y < zone.top() + 15.0 {
            ui.scroll_with_delta(egui::vec2(0.0, 4.0));
        } else if souris.y > zone.bottom() - 15.0 {
            ui.scroll_with_delta(egui::vec2(0.0, -4.0));
        }
        let vers = place_sous_la_souris(lignes, souris.y);
        // Un peu au-dessus ou au-dessous de la liste, ça compte encore : on
        // vise le haut ou le bas sans avoir à être précis.
        let dans_la_liste = zone.expand2(egui::vec2(0.0, 20.0)).contains(souris);
        if dans_la_liste {
            if let (Some(premiere), Some(derniere)) = (lignes.first(), lignes.last()) {
                let y = lignes.get(vers).map_or(derniere.bottom(), |r| r.top());
                let trait_ = ui.visuals().selection.stroke;
                ui.painter().hline(
                    premiere.left()..=premiere.right(),
                    y,
                    egui::Stroke::new(2.0_f32, trait_.color),
                );
            }
        }
        // Lâché hors de la liste, on ne déplace rien : il a pu être lâché
        // sur une playlist, qui s'en occupe.
        if dans_la_liste && ui.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            self.deplacer(de, vers);
        }
    }
}

/// La place où insérer un morceau lâché à la hauteur `y` : avant la
/// première ligne dont le milieu est plus bas que la souris, ou tout en bas.
fn place_sous_la_souris(lignes: &[egui::Rect], y: f32) -> usize {
    lignes
        .iter()
        .position(|r| y < r.center().y)
        .unwrap_or(lignes.len())
}

/// Un nombre au hasard entre 0 et n - 1 (n doit être au moins 1).
/// La bibliothèque standard de Rust mélange ses clés de hachage au hasard :
/// on s'en sert comme d'un dé, sans ajouter de bibliothèque.
fn hasard(n: usize) -> usize {
    (RandomState::new().hash_one(0u8) % n as u64) as usize
}

/// Une rangée d'une longue liste qu'on fait défiler. Si elle est hors de
/// l'écran, on réserve juste sa place, sans rien dessiner : avec des milliers
/// de morceaux, dessiner toutes les lignes à chaque image coûterait cher.
/// `hauteur` est celle d'une rangée, mesurée sur la dernière dessinée (rien au
/// tout début : on dessine alors). `n` donne à la rangée un identifiant qui
/// ne change pas quand on fait défiler. Renvoie la place de la rangée et ce
/// que `contenu` a renvoyé (rien si elle n'a pas été dessinée).
pub fn rangee<R>(
    ui: &mut egui::Ui,
    hauteur: &mut Option<f32>,
    n: impl std::hash::Hash,
    contenu: impl FnOnce(&mut egui::Ui) -> R,
) -> (egui::Rect, Option<R>) {
    if let Some(h) = *hauteur {
        let taille = egui::vec2(ui.available_width(), h);
        if !ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, taille)) {
            let (place, _) = ui.allocate_exact_size(taille, egui::Sense::hover());
            return (place, None);
        }
    }
    let dessinee = ui.push_id(n, |ui| ui.horizontal(contenu).inner);
    let place = dessinee.response.rect;
    *hauteur = Some(place.height());
    (place, Some(dessinee.inner))
}

/// Le nom d'un fichier sans son dossier, par exemple « chanson.mp3 ».
pub fn nom_du_fichier(chemin: &Path) -> String {
    chemin
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn liste(n: usize) -> ListeDeLecture {
        let mut l = ListeDeLecture::default();
        l.ajouter((0..n).map(|i| PathBuf::from(format!("{i}.mp3"))).collect());
        l
    }

    #[test]
    fn suivant_et_precedent() {
        let mut l = liste(3);
        assert_eq!(l.suivant(), Some(0));
        l.choisir(1);
        assert_eq!(l.suivant(), Some(2));
        assert_eq!(l.precedent(), Some(0));
        l.choisir(2);
        assert_eq!(l.suivant(), None);
        l.choisir(0);
        assert_eq!(l.precedent(), Some(0));
        assert_eq!(liste(0).precedent(), None);
    }

    #[test]
    fn aleatoire_joue_chaque_morceau_une_fois() {
        let mut l = liste(5);
        l.choisir(2);
        l.changer_aleatoire(true);
        let mut joues = vec![2];
        while let Some(n) = l.suivant() {
            assert!(!joues.contains(&n), "{n} joué deux fois");
            l.choisir(n);
            joues.push(n);
        }
        joues.sort();
        assert_eq!(joues, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn aleatoire_precedent_revient_en_arriere() {
        let mut l = liste(5);
        l.changer_aleatoire(true);
        let a = l.suivant().unwrap();
        l.choisir(a);
        let b = l.suivant().unwrap();
        l.choisir(b);
        assert_eq!(l.precedent(), Some(a));
        l.choisir(a);
        // Au premier morceau de l'historique, on le relance.
        assert_eq!(l.precedent(), Some(a));
    }

    #[test]
    fn repetition_repart_du_debut() {
        let mut l = liste(3);
        l.changer_repetition(true);
        l.choisir(2);
        assert_eq!(l.suivant(), Some(0));
        l.changer_repetition(false);
        assert_eq!(l.suivant(), None);
        assert_eq!(liste(0).suivant(), None);
        let mut vide = liste(0);
        vide.changer_repetition(true);
        assert_eq!(vide.suivant(), None);
    }

    #[test]
    fn repetition_aleatoire_refait_des_tours_complets() {
        let mut l = liste(4);
        l.changer_aleatoire(true);
        l.changer_repetition(true);
        let mut joues = Vec::new();
        for _ in 0..12 {
            let n = l.suivant().unwrap();
            assert_ne!(Some(n), l.courant(), "même morceau deux fois de suite");
            l.choisir(n);
            joues.push(n);
        }
        // Trois tours de 4 : chaque tour contient chaque morceau une fois.
        for tour in joues.chunks(4) {
            let mut tour = tour.to_vec();
            tour.sort();
            assert_eq!(tour, [0, 1, 2, 3]);
        }
    }

    #[test]
    fn repetition_un_seul_morceau() {
        let mut l = liste(1);
        l.changer_aleatoire(true);
        l.changer_repetition(true);
        l.choisir(0);
        assert_eq!(l.suivant(), Some(0));
    }

    #[test]
    fn hasard_donne_des_valeurs_variees() {
        let tirages: std::collections::HashSet<_> = (0..200).map(|_| hasard(10)).collect();
        assert!(tirages.len() > 5);
        assert!(tirages.iter().all(|&t| t < 10));
    }

    #[test]
    fn aleatoire_ajout_lance_un_morceau_au_hasard() {
        let mut l = liste(2);
        l.changer_aleatoire(true);
        let tirages: std::collections::HashSet<_> = (0..50)
            .map(|_| l.ajouter((0..10).map(|i| PathBuf::from(format!("{i}.mp3"))).collect()))
            .map(|n| n.unwrap() % 10)
            .collect();
        assert!(tirages.len() > 3);
    }

    #[test]
    fn retirer_decale_le_morceau_en_cours() {
        let mut l = liste(4);
        l.choisir(2);
        assert!(!l.retirer(0));
        assert_eq!(l.courant(), Some(1));
        assert!(!l.retirer(3));
        assert_eq!(l.courant(), Some(1));
        assert!(l.retirer(1));
        assert_eq!(l.courant(), None);
    }

    #[test]
    fn ajouter_renvoie_le_premier_ajoute() {
        let mut l = liste(2);
        assert_eq!(l.ajouter(vec![PathBuf::from("x.mp3")]), Some(2));
        assert_eq!(l.ajouter(vec![]), None);
    }

    /// Un simple clic (appui puis relâche) à la hauteur y, sur les noms des
    /// morceaux : renvoie le numéro du morceau à jouer, s'il y en a un.
    fn cliquer_a(
        l: &mut ListeDeLecture,
        ctx: &egui::Context,
        tags: &TagsConnus,
        temps: f64,
        y: f32,
    ) -> Option<usize> {
        let pos = egui::pos2(70.0, y);
        let mut jouer = None;
        for (n, pressed) in [false, true].into_iter().enumerate() {
            let entree = egui::RawInput {
                time: Some(temps + n as f64 * 0.05),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: !pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(entree, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(Action::Jouer(n)) = l.afficher(ui, tags, &[]) {
                        jouer = Some(n);
                    }
                });
            });
        }
        jouer
    }

    /// Avec une très longue liste, seules les lignes à l'écran sont dessinées :
    /// on vérifie que la liste défile bien jusqu'au morceau en cours, même
    /// très loin, et qu'on peut cliquer sur les lignes qu'on y voit.
    #[test]
    fn une_longue_liste_defile_jusqu_au_morceau_en_cours() {
        let mut l = liste(3000);
        l.choisir(2500);
        let ctx = egui::Context::default();
        let tags = TagsConnus::new(&ctx);
        // Quelques images, le temps que le défilement (animé) arrive.
        for i in 0..40 {
            let entree = egui::RawInput {
                time: Some(i as f64 * 0.05),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(entree, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    l.afficher(ui, &tags, &[]);
                });
            });
        }
        let n = cliquer_a(&mut l, &ctx, &tags, 10.0, 20.0).expect("aucune ligne sous la souris");
        assert!((2400..=2500).contains(&n), "ligne {n}");
        // Et la ligne d'en dessous est bien la suivante.
        let hauteur = l.hauteur_ligne.unwrap();
        let suivante = cliquer_a(&mut l, &ctx, &tags, 20.0, 20.0 + hauteur);
        assert_eq!(suivante, Some(n + 1));
    }

    #[test]
    fn un_simple_clic_joue_le_morceau() {
        let mut l = liste(3);
        let ctx = egui::Context::default();
        let tags = TagsConnus::new(&ctx);
        // On balaie la liste de haut en bas, un clic tous les 2 pixels :
        // chaque morceau doit être lancé, dans l'ordre, d'un seul clic.
        let mut joues = Vec::new();
        for i in 0..100 {
            if let Some(n) = cliquer_a(&mut l, &ctx, &tags, i as f64, i as f32 * 2.0) {
                if joues.last() != Some(&n) {
                    joues.push(n);
                }
            }
        }
        assert_eq!(joues, vec![0, 1, 2]);
    }

    /// Les noms des morceaux, dans l'ordre de la liste.
    fn noms(l: &ListeDeLecture) -> Vec<String> {
        l.morceaux().iter().map(|m| nom_du_fichier(m)).collect()
    }

    #[test]
    fn deplacer_garde_le_morceau_en_cours() {
        let mut l = liste(5);
        l.choisir(1);
        l.deplacer(0, 5); // le premier tout en bas
        assert_eq!(noms(&l), ["1.mp3", "2.mp3", "3.mp3", "4.mp3", "0.mp3"]);
        assert_eq!(l.courant(), Some(0));
        l.deplacer(3, 0); // 4.mp3 tout en haut
        assert_eq!(noms(&l), ["4.mp3", "1.mp3", "2.mp3", "3.mp3", "0.mp3"]);
        assert_eq!(l.courant(), Some(1));
        l.deplacer(1, 4); // le morceau en cours avant le dernier
        assert_eq!(noms(&l), ["4.mp3", "2.mp3", "3.mp3", "1.mp3", "0.mp3"]);
        assert_eq!(l.courant(), Some(3));
        assert_eq!(l.suivant(), Some(4));
        // Lâché sur place : rien ne change.
        l.deplacer(2, 2);
        l.deplacer(2, 3);
        assert_eq!(noms(&l), ["4.mp3", "2.mp3", "3.mp3", "1.mp3", "0.mp3"]);
    }

    #[test]
    fn lire_ensuite_place_le_morceau_apres_celui_en_cours() {
        let mut l = liste(5);
        l.choisir(1);
        l.lire_ensuite(4);
        assert_eq!(noms(&l), ["0.mp3", "1.mp3", "4.mp3", "2.mp3", "3.mp3"]);
        assert_eq!(l.suivant(), Some(2));
        l.lire_ensuite(0); // un morceau placé avant celui en cours
        assert_eq!(noms(&l), ["1.mp3", "0.mp3", "4.mp3", "2.mp3", "3.mp3"]);
        assert_eq!(l.courant(), Some(0));
        assert_eq!(l.suivant(), Some(1));
        // Sur le morceau en cours : rien ne change.
        l.lire_ensuite(0);
        assert_eq!(noms(&l), ["1.mp3", "0.mp3", "4.mp3", "2.mp3", "3.mp3"]);
    }

    #[test]
    fn lire_ensuite_sans_morceau_en_cours_remonte_en_tete() {
        let mut l = liste(3);
        l.lire_ensuite(2);
        assert_eq!(noms(&l), ["2.mp3", "0.mp3", "1.mp3"]);
        assert_eq!(l.suivant(), Some(0));
    }

    #[test]
    fn lire_ensuite_passe_avant_le_hasard() {
        for _ in 0..20 {
            let mut l = liste(8);
            l.changer_aleatoire(true);
            l.choisir(3);
            l.lire_ensuite(6);
            assert_eq!(l.suivant(), Some(4));
            l.choisir(4);
            // Ensuite, le hasard reprend parmi les morceaux pas encore joués.
            let n = l.suivant().unwrap();
            assert!(n != 3 && n != 4);
        }
    }

    #[test]
    fn retirer_le_morceau_a_lire_ensuite_l_oublie() {
        let mut l = liste(4);
        l.changer_aleatoire(true);
        l.choisir(0);
        l.lire_ensuite(3);
        l.retirer(1);
        let n = l.suivant().unwrap();
        assert!(n == 1 || n == 2);
    }

    /// Une image de la liste avec ces gestes de souris (à un instant donné).
    fn image(
        l: &mut ListeDeLecture,
        ctx: &egui::Context,
        tags: &TagsConnus,
        temps: f64,
        events: Vec<egui::Event>,
    ) -> Option<Action> {
        let entree = egui::RawInput {
            time: Some(temps),
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let mut action = None;
        let _ = ctx.run(entree, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = l.afficher(ui, tags, &[]);
            });
        });
        action
    }

    fn bouton(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    /// Glisse le morceau à la hauteur `de` jusqu'à la position `vers`, en
    /// plusieurs petits pas, puis lâche. Renvoie les actions demandées.
    fn glisser_de(l: &mut ListeDeLecture, de: f32, vers: egui::Pos2) -> Vec<Action> {
        let ctx = egui::Context::default();
        let tags = TagsConnus::new(&ctx);
        let mut actions = Vec::new();
        let depart = egui::pos2(70.0, de);
        let mut t = 0.0;
        let mut pas = |l: &mut ListeDeLecture, events| {
            t += 0.02;
            actions.extend(image(l, &ctx, &tags, t, events));
        };
        pas(l, vec![]);
        pas(l, bouton(depart, true));
        for i in 1..=20 {
            let p = depart + (vers - depart) * (i as f32 / 20.0);
            pas(l, vec![egui::Event::PointerMoved(p)]);
        }
        pas(l, bouton(vers, false));
        pas(l, vec![]);
        actions
    }

    #[test]
    fn glisser_un_morceau_change_sa_place() {
        // Les lignes font une vingtaine de pixels de haut : on attrape la
        // première et on la lâche tout en bas de la liste.
        let mut l = liste(4);
        let actions = glisser_de(&mut l, 15.0, egui::pos2(70.0, 4.0 * 21.0 + 2.0));
        assert!(actions.is_empty(), "un glisser ne lance pas de morceau");
        assert_eq!(noms(&l), ["1.mp3", "2.mp3", "3.mp3", "0.mp3"]);
        // Et le dernier, on le remonte en haut.
        let actions = glisser_de(&mut l, 3.0 * 21.0 + 10.0, egui::pos2(70.0, 3.0));
        assert!(actions.is_empty());
        assert_eq!(noms(&l), ["0.mp3", "1.mp3", "2.mp3", "3.mp3"]);
    }

    /// Dans une très longue liste (lignes hors de l'écran non dessinées),
    /// glisser un morceau donne la même chose que dans une petite liste.
    #[test]
    fn glisser_dans_une_longue_liste() {
        let mut petite = liste(6);
        let mut longue = liste(1000);
        glisser_de(&mut petite, 15.0, egui::pos2(70.0, 3.0 * 21.0 + 2.0));
        glisser_de(&mut longue, 15.0, egui::pos2(70.0, 3.0 * 21.0 + 2.0));
        assert_ne!(noms(&petite)[0], "0.mp3", "le morceau a bougé");
        assert_eq!(noms(&longue)[..5], noms(&petite)[..5]);
        assert_eq!(noms(&longue).len(), 1000);
    }

    #[test]
    fn lacher_hors_de_la_liste_ne_change_rien() {
        let mut l = liste(4);
        glisser_de(&mut l, 15.0, egui::pos2(70.0, 290.0));
        assert_eq!(noms(&l), ["0.mp3", "1.mp3", "2.mp3", "3.mp3"]);
    }

    #[test]
    fn un_morceau_glisse_hors_de_la_liste_emporte_son_fichier() {
        // Hors de la liste, c'est une playlist qui pourra le recevoir : le
        // morceau glissé doit donc transporter son fichier.
        let mut l = liste(3);
        let ctx = egui::Context::default();
        let tags = TagsConnus::new(&ctx);
        let depart = egui::pos2(70.0, 36.0); // le 2e morceau
        image(&mut l, &ctx, &tags, 0.0, vec![]);
        image(&mut l, &ctx, &tags, 0.02, bouton(depart, true));
        for i in 1..=10 {
            let p = depart + egui::vec2(0.0, 25.0 * i as f32);
            image(
                &mut l,
                &ctx,
                &tags,
                0.02 + 0.02 * i as f64,
                vec![egui::Event::PointerMoved(p)],
            );
        }
        let fichier = egui::DragAndDrop::payload::<MorceauGlisse>(&ctx).map(|m| m.fichier.clone());
        assert_eq!(fichier, Some(PathBuf::from("1.mp3")));
    }
}
