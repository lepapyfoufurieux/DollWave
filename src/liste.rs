//! La liste de lecture : les morceaux à jouer les uns après les autres,
//! et lequel est en cours.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::format_temps;
use crate::tags::TagsConnus;

/// Hauteur maximale de la liste à l'écran ; au-delà, on fait défiler.
const HAUTEUR_MAX: f32 = 160.0;

/// Ce que l'utilisateur a demandé en cliquant dans la liste.
pub enum Action {
    /// Jouer le morceau numéro n (clic sur sa ligne).
    Jouer(usize),
    /// Retirer le morceau numéro n de la liste (bouton ✖).
    Retirer(usize),
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
    }

    pub fn aleatoire(&self) -> bool {
        self.aleatoire
    }

    /// Active ou désactive la lecture aléatoire. En l'activant, on repart
    /// pour un nouveau tour : seul le morceau en cours compte comme déjà joué.
    pub fn changer_aleatoire(&mut self, actif: bool) {
        self.aleatoire = actif;
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

    /// Dessine la liste. Renvoie ce que l'utilisateur a demandé, s'il a cliqué.
    /// Chaque morceau est nommé d'après ses tags (« Artiste – Titre »), avec
    /// sa durée à droite.
    pub fn afficher(&mut self, ui: &mut egui::Ui, tags: &TagsConnus) -> Option<Action> {
        let mut action = None;
        // On ne défile que quand le morceau change : entre-temps, on peut
        // faire défiler la liste à la main sans qu'elle revienne toute seule.
        let defiler = self.courant != self.courant_affiche;
        self.courant_affiche = self.courant;
        egui::ScrollArea::vertical()
            .max_height(HAUTEUR_MAX)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (n, fichier) in self.morceaux.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("✖").on_hover_text("Retirer").clicked() {
                            action = Some(Action::Retirer(n));
                        }
                        let en_cours = self.courant == Some(n);
                        let texte = format!("{}. {}", n + 1, tags.libelle(fichier));
                        let ligne = ui
                            .selectable_label(en_cours, texte)
                            .on_hover_text(tags.infobulle(fichier));
                        if let Some(duree) = tags.duree(fichier) {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| ui.weak(format_temps(duree)),
                            );
                        }
                        if en_cours && defiler {
                            ligne.scroll_to_me(None);
                        }
                        // Un clic suffit. Pour egui, un double-clic, c'est deux
                        // clics : le second tombe sur le morceau devenu le
                        // morceau en cours, ce qui ne le relance pas (voir main.rs).
                        if ligne.clicked() {
                            action = Some(Action::Jouer(n));
                        }
                    });
                }
            });
        action
    }
}

/// Un nombre au hasard entre 0 et n - 1 (n doit être au moins 1).
/// La bibliothèque standard de Rust mélange ses clés de hachage au hasard :
/// on s'en sert comme d'un dé, sans ajouter de bibliothèque.
fn hasard(n: usize) -> usize {
    (RandomState::new().hash_one(0u8) % n as u64) as usize
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
        let pos = egui::pos2(45.0, y);
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
                    if let Some(Action::Jouer(n)) = l.afficher(ui, tags) {
                        jouer = Some(n);
                    }
                });
            });
        }
        jouer
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
}
