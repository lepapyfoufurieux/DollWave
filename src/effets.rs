//! Les effets du thème néon (étape 8) : lignes d'écran façon vieux moniteur,
//! lueurs qui respirent pendant la lecture (en pause, tout se fige), et le
//! réglage « Intensité des effets » du menu ⚙ Options.
//!
//! Les deux réglages sont mémorisés dans le dossier de réglages
//! (lignes_ecran.txt : « oui » ou « non » ; intensite_effets.txt : un nombre).

use eframe::egui;

use crate::bibliotheque::memoriser;
use crate::systeme::fichier_de_reglage;
use crate::theme;

/// Le fichier où l'on mémorise si les lignes d'écran sont affichées.
const REGLAGE_LIGNES: &str = "lignes_ecran.txt";
/// Le fichier où l'on mémorise l'intensité des effets (de 0 à 2).
const REGLAGE_INTENSITE: &str = "intensite_effets.txt";
/// La durée d'une respiration des lueurs, en secondes.
const PERIODE: f32 = 2.4;
/// Lignes d'écran sur toute la fenêtre : une ligne sombre toutes les 4 points,
/// très légère (comme sur la maquette).
const ECART_FENETRE: f32 = 4.0;
const OPACITE_FENETRE: f32 = 31.0;

pub struct Effets {
    /// Les lignes d'écran sont-elles affichées ?
    pub lignes: bool,
    /// La force des lueurs et des lignes, de 0 (aucun effet) à 2.
    pub intensite: f32,
    /// Où en est la respiration des lueurs (en secondes).
    phase: f32,
}

impl Effets {
    /// Les effets tels qu'on les a laissés au dernier lancement.
    pub fn charger() -> Self {
        let lire = |nom| fichier_de_reglage(nom).and_then(|f| std::fs::read_to_string(f).ok());
        let effets = Self {
            lignes: lignes_depuis(lire(REGLAGE_LIGNES).as_deref()),
            intensite: intensite_depuis(lire(REGLAGE_INTENSITE).as_deref()),
            phase: 0.0,
        };
        effets.appliquer();
        effets
    }

    /// Transmet les réglages au thème, qui dessine les lueurs et les lignes.
    fn appliquer(&self) {
        theme::fixer_intensite(self.intensite);
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

    /// Les lignes d'écran par-dessus toute la fenêtre (mais sous les menus
    /// et les fenêtres, qui restent bien nets).
    pub fn dessiner(&self, ctx: &egui::Context) {
        let calque =
            egui::LayerId::new(egui::Order::PanelResizeLine, egui::Id::new("lignes_ecran"));
        let peintre = ctx.layer_painter(calque);
        theme::lignes_d_ecran(&peintre, ctx.screen_rect(), ECART_FENETRE, OPACITE_FENETRE);
    }

    /// Les réglages des effets, dans le menu ⚙ Options.
    pub fn menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .checkbox(&mut self.lignes, "Lignes d'écran")
            .on_hover_text("De fines lignes sombres, comme sur un vieux moniteur")
            .changed()
        {
            memoriser(REGLAGE_LIGNES, if self.lignes { "oui" } else { "non" });
            self.appliquer();
        }
        ui.horizontal(|ui| {
            ui.label("Intensité des effets");
            let curseur = egui::Slider::new(&mut self.intensite, 0.0..=2.0)
                .step_by(0.25)
                .custom_formatter(|v, _| format!("{:.0} %", v * 100.0));
            if ui
                .add(curseur)
                .on_hover_text("La force des lueurs et des lignes d'écran (0 % : aucun effet)")
                .changed()
            {
                memoriser(REGLAGE_INTENSITE, &self.intensite.to_string());
                self.appliquer();
            }
        });
    }
}

/// La respiration des lueurs à ce moment de la période : de 1 (au début)
/// à 0 (à la moitié), puis de nouveau 1, en douceur.
fn respiration(phase: f32) -> f32 {
    0.5 + 0.5 * (phase / PERIODE * std::f32::consts::TAU).cos()
}

/// Lit le fichier des lignes d'écran : affichées sauf si l'on a mis « non ».
fn lignes_depuis(texte: Option<&str>) -> bool {
    texte.is_none_or(|t| t.trim() != "non")
}

/// Lit le fichier de l'intensité : 1 par défaut (fichier absent ou illisible).
fn intensite_depuis(texte: Option<&str>) -> f32 {
    texte
        .and_then(|t| t.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .map_or(1.0, |v| v.clamp(0.0, 2.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_reglages_se_relisent() {
        assert!(lignes_depuis(None));
        assert!(lignes_depuis(Some("oui\n")));
        assert!(!lignes_depuis(Some("non\n")));
        assert_eq!(intensite_depuis(None), 1.0);
        assert_eq!(intensite_depuis(Some("1.5\n")), 1.5);
        assert_eq!(intensite_depuis(Some("7")), 2.0);
        assert_eq!(intensite_depuis(Some("-1")), 0.0);
        assert_eq!(intensite_depuis(Some("NaN")), 1.0);
        assert_eq!(intensite_depuis(Some("beaucoup")), 1.0);
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
            intensite: 1.0,
            phase: 0.0,
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
