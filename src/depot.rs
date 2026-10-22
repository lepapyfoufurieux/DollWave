//! Le glisser-déposer : des MP3 ou des dossiers lâchés dans la fenêtre
//! depuis l'Explorateur Windows.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;

use crate::bibliotheque::{est_un_morceau, morceaux_du_dossier};

/// Les fichiers et dossiers que l'utilisateur vient de lâcher dans la
/// fenêtre (vide la plupart du temps).
pub fn elements_deposes(ctx: &egui::Context) -> Vec<PathBuf> {
    ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .filter_map(|f| f.path.clone())
            .collect()
    })
}

/// Les MP3 de ce qui a été déposé, dans l'ordre : les fichiers MP3 tels
/// quels, et pour chaque dossier ses MP3 (sous-dossiers compris) rangés
/// comme dans la bibliothèque. Les autres fichiers sont ignorés.
pub fn mp3_deposes(elements: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut fichiers = Vec::new();
    for element in elements {
        if element.is_dir() {
            fichiers.extend(morceaux_du_dossier(&element));
        } else if est_un_morceau(&element) {
            fichiers.push(element);
        }
    }
    fichiers
}

/// Les dépôts en cours de lecture. Parcourir un gros dossier (des milliers
/// de fichiers, ou un disque réseau) prend du temps : on le fait en
/// arrière-plan pour que la fenêtre ne se fige pas.
#[derive(Default)]
pub struct Depots {
    /// Un récepteur par dépôt, dans l'ordre où ils ont été lâchés.
    en_cours: VecDeque<Receiver<Vec<PathBuf>>>,
}

impl Depots {
    /// Commence à lire ce qui vient d'être lâché.
    pub fn lancer(&mut self, ctx: &egui::Context, elements: Vec<PathBuf>) {
        let (envoi, reception) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = envoi.send(mp3_deposes(elements));
            ctx.request_repaint(); // pour que l'appli prenne le résultat tout de suite
        });
        self.en_cours.push_back(reception);
    }

    /// Les MP3 du plus ancien dépôt, s'il est prêt (les dépôts arrivent
    /// dans l'ordre où ils ont été lâchés).
    pub fn recu(&mut self) -> Option<Vec<PathBuf>> {
        match self.en_cours.front()?.try_recv() {
            Ok(fichiers) => {
                self.en_cours.pop_front();
                Some(fichiers)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.en_cours.pop_front();
                Some(Vec::new())
            }
        }
    }

    pub fn en_cours(&self) -> bool {
        !self.en_cours.is_empty()
    }
}

/// Pendant qu'on survole la fenêtre avec des fichiers, on l'assombrit et on
/// affiche un texte, pour montrer qu'on peut les lâcher.
pub fn dessiner_survol(ctx: &egui::Context) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let calque = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("survol_depot"));
    let peintre = ctx.layer_painter(calque);
    let ecran = ctx.screen_rect();
    peintre.rect_filled(ecran, 0.0, egui::Color32::from_black_alpha(180));
    peintre.text(
        ecran.center(),
        egui::Align2::CENTER_CENTER,
        "Déposez ici pour ajouter à la liste",
        egui::FontId::proportional(22.0),
        egui::Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liste::nom_du_fichier;

    #[test]
    fn depot_melange_fichiers_et_dossiers() {
        let racine = std::env::temp_dir().join("lecteur_mp3_test_depot");
        let _ = std::fs::remove_dir_all(&racine);
        std::fs::create_dir_all(racine.join("album/cd2")).unwrap();
        for f in ["seul.mp3", "image.jpg", "album/z.mp3", "album/cd2/a.mp3"] {
            std::fs::write(racine.join(f), b"").unwrap();
        }
        let depose = vec![
            racine.join("seul.mp3"),
            racine.join("image.jpg"),
            racine.join("album"),
        ];
        let noms: Vec<_> = mp3_deposes(depose)
            .iter()
            .map(|f| nom_du_fichier(f))
            .collect();
        assert_eq!(noms, ["seul.mp3", "a.mp3", "z.mp3"]);
        // Rien d'utilisable : aucun morceau.
        assert!(mp3_deposes(vec![racine.join("image.jpg")]).is_empty());
        std::fs::remove_dir_all(&racine).unwrap();
    }

    #[test]
    fn les_depots_se_lisent_en_arriere_plan_dans_l_ordre() {
        let racine = std::env::temp_dir().join("lecteur_mp3_test_depots");
        let _ = std::fs::remove_dir_all(&racine);
        std::fs::create_dir_all(racine.join("gros")).unwrap();
        for i in 0..300 {
            std::fs::write(racine.join("gros").join(format!("{i:03}.mp3")), b"").unwrap();
        }
        std::fs::write(racine.join("seul.mp3"), b"").unwrap();
        let ctx = egui::Context::default();
        let mut depots = Depots::default();
        depots.lancer(&ctx, vec![racine.join("gros")]);
        depots.lancer(&ctx, vec![racine.join("seul.mp3")]);
        assert!(depots.en_cours());
        let mut recus = Vec::new();
        let debut = std::time::Instant::now();
        while depots.en_cours() && debut.elapsed() < std::time::Duration::from_secs(5) {
            if let Some(fichiers) = depots.recu() {
                recus.push(fichiers.len());
            }
        }
        assert_eq!(recus, [300, 1]);
        std::fs::remove_dir_all(&racine).unwrap();
    }
}
