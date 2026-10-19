//! La liste de lecture : les morceaux à jouer les uns après les autres,
//! et lequel est en cours.

use std::path::{Path, PathBuf};

use eframe::egui;

/// Hauteur maximale de la liste à l'écran ; au-delà, on fait défiler.
const HAUTEUR_MAX: f32 = 160.0;

/// Ce que l'utilisateur a demandé en cliquant dans la liste.
pub enum Action {
    /// Jouer le morceau numéro n (double-clic sur sa ligne).
    Jouer(usize),
    /// Retirer le morceau numéro n de la liste (bouton ✖).
    Retirer(usize),
}

#[derive(Default)]
pub struct ListeDeLecture {
    morceaux: Vec<PathBuf>,
    /// Le numéro du morceau en cours dans la liste, s'il y en a un.
    courant: Option<usize>,
}

impl ListeDeLecture {
    /// Ajoute des fichiers à la fin de la liste. Renvoie le numéro du premier
    /// ajouté (pour pouvoir le lancer), ou rien si la liste n'a pas grandi.
    pub fn ajouter(&mut self, fichiers: Vec<PathBuf>) -> Option<usize> {
        let premier = self.morceaux.len();
        self.morceaux.extend(fichiers);
        (self.morceaux.len() > premier).then_some(premier)
    }

    /// Ajoute tous les MP3 d'un dossier et de ses sous-dossiers, triés par nom.
    pub fn ajouter_dossier(&mut self, dossier: &Path) -> Option<usize> {
        let mut fichiers = Vec::new();
        chercher_mp3(dossier, &mut fichiers);
        fichiers.sort_by_key(|f| f.to_string_lossy().to_lowercase());
        self.ajouter(fichiers)
    }

    /// Retire un morceau. Le numéro du morceau en cours est décalé si besoin.
    /// Renvoie `true` si c'est le morceau en cours qui a été retiré.
    pub fn retirer(&mut self, n: usize) -> bool {
        if n >= self.morceaux.len() {
            return false;
        }
        self.morceaux.remove(n);
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
        self.courant = None;
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
    }

    /// Le numéro du morceau suivant, ou rien si on est au bout de la liste.
    pub fn suivant(&self) -> Option<usize> {
        let n = self.courant.map_or(0, |c| c + 1);
        (n < self.morceaux.len()).then_some(n)
    }

    /// Le numéro du morceau précédent. Au début de la liste, on reste sur le
    /// premier (il sera relancé depuis le début).
    pub fn precedent(&self) -> Option<usize> {
        if self.morceaux.is_empty() {
            return None;
        }
        Some(self.courant.map_or(0, |c| c.saturating_sub(1)))
    }

    /// Dessine la liste. Renvoie ce que l'utilisateur a demandé, s'il a cliqué.
    pub fn afficher(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
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
                        let texte = format!("{}. {}", n + 1, nom_du_fichier(fichier));
                        let ligne = ui.selectable_label(en_cours, texte);
                        if ligne.double_clicked() {
                            action = Some(Action::Jouer(n));
                        }
                    });
                }
            });
        action
    }
}

/// Le nom d'un fichier sans son dossier, par exemple « chanson.mp3 ».
pub fn nom_du_fichier(chemin: &Path) -> String {
    chemin
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Parcourt un dossier (et ses sous-dossiers) et note tous les fichiers .mp3.
/// Les dossiers illisibles sont simplement ignorés.
fn chercher_mp3(dossier: &Path, trouves: &mut Vec<PathBuf>) {
    let Ok(contenu) = std::fs::read_dir(dossier) else {
        return;
    };
    for entree in contenu.flatten() {
        let chemin = entree.path();
        if chemin.is_dir() {
            chercher_mp3(&chemin, trouves);
        } else if chemin
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
        {
            trouves.push(chemin);
        }
    }
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

    #[test]
    fn dossier_trie_et_sous_dossiers() {
        let racine = std::env::temp_dir().join("lecteur_mp3_test_dossier");
        let _ = std::fs::remove_dir_all(&racine);
        std::fs::create_dir_all(racine.join("b")).unwrap();
        for f in ["c.mp3", "a.MP3", "notes.txt", "b/d.mp3"] {
            std::fs::write(racine.join(f), b"").unwrap();
        }
        let mut l = ListeDeLecture::default();
        l.ajouter_dossier(&racine);
        let noms: Vec<_> = (0..3).map(|i| nom_du_fichier(l.fichier(i).unwrap())).collect();
        assert_eq!(noms, ["a.MP3", "d.mp3", "c.mp3"]);
        assert!(l.fichier(3).is_none());
        std::fs::remove_dir_all(&racine).unwrap();
    }
}
