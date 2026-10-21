//! Les effets du thème néon (étape 8) : lignes d'écran façon vieux moniteur,
//! et lueurs qui respirent pendant la lecture (en pause, tout se fige).

use eframe::egui;

use crate::theme;

/// La durée d'une respiration des lueurs, en secondes.
const PERIODE: f32 = 2.4;
/// Lignes d'écran sur toute la fenêtre : une ligne sombre toutes les 4 points,
/// très légère (comme sur la maquette).
const ECART_FENETRE: f32 = 4.0;
const OPACITE_FENETRE: f32 = 31.0;

#[derive(Default)]
pub struct Effets {
    /// Où en est la respiration des lueurs (en secondes).
    phase: f32,
}

impl Effets {
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
}

/// La respiration des lueurs à ce moment de la période : de 1 (au début)
/// à 0 (à la moitié), puis de nouveau 1, en douceur.
fn respiration(phase: f32) -> f32 {
    0.5 + 0.5 * (phase / PERIODE * std::f32::consts::TAU).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_respiration_va_de_1_a_0_et_revient() {
        assert!((respiration(0.0) - 1.0).abs() < 1e-6);
        assert!(respiration(PERIODE / 2.0) < 1e-6);
        assert!((respiration(PERIODE) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn en_pause_les_lueurs_ne_bougent_plus() {
        let mut effets = Effets::default();
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
