//! Les effets du thème néon (étape 8) : lignes d'écran façon vieux moniteur,
//! et lueurs qui respirent pendant la lecture (en pause, tout se fige).
//!
//! Les lignes d'écran se cochent dans ⚙ Options ; le choix est mémorisé
//! dans le dossier de réglages (lignes_ecran.txt : « oui » ou « non »).

use std::cell::Cell;

use eframe::egui::{self, Rect};

use crate::bibliotheque::memoriser;
use crate::systeme::fichier_de_reglage;
use crate::theme;

/// Le fichier où l'on mémorise si les lignes d'écran sont affichées.
const REGLAGE_LIGNES: &str = "lignes_ecran.txt";
/// La durée d'une respiration des lueurs, en secondes.
const PERIODE: f32 = 2.4;
/// Lignes d'écran sur toute la fenêtre : une ligne sombre toutes les 4 points,
/// très légère (comme sur la maquette).
const ECART_FENETRE: f32 = 4.0;
const OPACITE_FENETRE: f32 = 31.0;

pub struct Effets {
    /// Les lignes d'écran sont-elles affichées ?
    pub lignes: bool,
    /// Où en est la respiration des lueurs (en secondes).
    phase: f32,
    /// Une zone où les lignes de la fenêtre ne passent pas (la pochette :
    /// elle a ses propres lignes, sous les paroles). Remise à zéro à chaque image.
    sans_lignes: Cell<Option<Rect>>,
}

impl Effets {
    /// Les effets tels qu'on les a laissés au dernier lancement.
    pub fn charger() -> Self {
        let lire = |nom| fichier_de_reglage(nom).and_then(|f| std::fs::read_to_string(f).ok());
        let effets = Self {
            lignes: lignes_depuis(lire(REGLAGE_LIGNES).as_deref()),
            phase: 0.0,
            sans_lignes: Cell::new(None),
        };
        effets.appliquer();
        effets
    }

    /// Transmet le réglage au thème, qui dessine les lignes.
    fn appliquer(&self) {
        theme::fixer_lignes(self.lignes);
    }

    /// À chaque image : la respiration des lueurs avance seulement pendant
    /// la lecture. Sinon, elles restent allumées en plein, sans bouger.
    pub fn avancer(&mut self, ecoule: f32, en_lecture: bool) {
        if en_lecture {
            self.phase = (self.phase + ecoule) % PERIODE;
            theme::fixer_pulsation(respiration(self.phase));
        } else {
            theme::fixer_pulsation(1.0);
        }
    }

    /// Les lignes de la fenêtre ne passeront pas sur `zone` à cette image.
    pub fn proteger(&self, zone: Rect) {
        self.sans_lignes.set(Some(zone));
    }

    /// Les lignes d'écran par-dessus toute la fenêtre (mais sous les menus
    /// et les fenêtres, qui restent bien nets), sauf sur la zone protégée.
    /// À appeler après les panneaux.
    pub fn dessiner(&self, ctx: &egui::Context) {
        let calque =
            egui::LayerId::new(egui::Order::PanelResizeLine, egui::Id::new("lignes_ecran"));
        let peintre = ctx.layer_painter(calque);
        for zone in autour(ctx.screen_rect(), self.sans_lignes.take()) {
            theme::lignes_d_ecran(&peintre, zone, ECART_FENETRE, OPACITE_FENETRE);
        }
    }

    /// Le réglage des effets, dans le menu ⚙ Options.
    pub fn menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .checkbox(&mut self.lignes, "Lignes d'écran")
            .on_hover_text("De fines lignes sombres, comme sur un vieux moniteur")
            .changed()
        {
            memoriser(REGLAGE_LIGNES, if self.lignes { "oui" } else { "non" });
            self.appliquer();
        }
    }
}

/// La respiration des lueurs à ce moment de la période : de 1 (au début)
/// à 0 (à la moitié), puis de nouveau 1, en douceur.
fn respiration(phase: f32) -> f32 {
    0.5 + 0.5 * (phase / PERIODE * std::f32::consts::TAU).cos()
}

/// Les morceaux de `ecran` autour de `trou` (au plus 4 bandes : dessus,
/// dessous, gauche, droite). Tout l'écran s'il n'y a pas de trou.
fn autour(ecran: Rect, trou: Option<Rect>) -> Vec<Rect> {
    let Some(trou) = trou.map(|t| t.intersect(ecran)).filter(|t| t.is_positive()) else {
        return vec![ecran];
    };
    let haut = Rect::from_min_max(ecran.min, egui::pos2(ecran.right(), trou.top()));
    let bas = Rect::from_min_max(egui::pos2(ecran.left(), trou.bottom()), ecran.max);
    let gauche = Rect::from_min_max(
        egui::pos2(ecran.left(), trou.top()),
        egui::pos2(trou.left(), trou.bottom()),
    );
    let droite = Rect::from_min_max(
        egui::pos2(trou.right(), trou.top()),
        egui::pos2(ecran.right(), trou.bottom()),
    );
    [haut, bas, gauche, droite]
        .into_iter()
        .filter(|r| r.is_positive())
        .collect()
}

/// Lit le fichier des lignes d'écran : affichées sauf si l'on a mis « non ».
fn lignes_depuis(texte: Option<&str>) -> bool {
    texte.is_none_or(|t| t.trim() != "non")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_reglages_se_relisent() {
        assert!(lignes_depuis(None));
        assert!(lignes_depuis(Some("oui\n")));
        assert!(!lignes_depuis(Some("non\n")));
    }

    #[test]
    fn les_lignes_contournent_la_pochette() {
        let ecran = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
        assert_eq!(autour(ecran, None), vec![ecran]);
        let pochette = Rect::from_min_max(egui::pos2(60.0, 50.0), egui::pos2(90.0, 80.0));
        let bandes = autour(ecran, Some(pochette));
        assert_eq!(bandes.len(), 4);
        let surface: f32 = bandes.iter().map(|r| r.area()).sum();
        assert_eq!(
            surface,
            100.0 * 100.0 - 30.0 * 30.0,
            "tout sauf la pochette"
        );
        assert!(bandes.iter().all(|r| !r.intersects(pochette.shrink(0.5))));
        // Collée au bord : une bande de moins.
        let au_bord = Rect::from_min_max(egui::pos2(60.0, 50.0), egui::pos2(100.0, 100.0));
        assert_eq!(autour(ecran, Some(au_bord)).len(), 2);
    }

    #[test]
    fn la_respiration_va_de_1_a_0_et_revient() {
        assert!((respiration(0.0) - 1.0).abs() < 1e-6);
        assert!(respiration(PERIODE / 2.0) < 1e-6);
        assert!((respiration(PERIODE) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn en_pause_les_lueurs_ne_bougent_plus() {
        let mut effets = Effets {
            lignes: true,
            phase: 0.0,
            sans_lignes: Cell::new(None),
        };
        effets.avancer(PERIODE / 2.0, true);
        assert!(
            theme::pulsation() < 1e-6,
            "pendant la lecture, elle respire"
        );
        effets.avancer(0.5, false);
        assert_eq!(theme::pulsation(), 1.0, "en pause, lueur pleine et figée");
        assert_eq!(effets.phase, PERIODE / 2.0, "la phase n'a pas avancé");
    }
}
