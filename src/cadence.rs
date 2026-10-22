//! La cadence de l'affichage : pendant la lecture, on redessine la fenêtre
//! 30 fois par seconde (au lieu d'une fois par rafraîchissement de l'écran,
//! 60 ou 144 fois), pour ménager le processeur. Et 15 fois par seconde
//! quand la fenêtre n'est pas active.
//!
//! Et une mesure, à cocher dans ⚙ Options (pas mémorisée) : le nombre
//! d'images par seconde et le temps de processeur que prend chaque image.

use std::time::{Duration, Instant};

use eframe::egui;

use crate::theme;

/// Le nombre d'images dessinées par seconde pendant la lecture, quand la
/// fenêtre est active, puis quand on travaille dans une autre fenêtre.
pub const IMAGES_PAR_SECONDE: f32 = 30.0;
pub const IMAGES_PAR_SECONDE_INACTIVE: f32 = 15.0;

/// Combien attendre, à partir de `maintenant`, avant de dessiner l'image
/// suivante, pour que les images commencées à `debut` se suivent à la cadence
/// voulue (zéro si l'on est déjà en retard). `active` : la fenêtre est-elle
/// celle où l'on travaille ?
pub fn attente(debut: Instant, maintenant: Instant, active: bool) -> Duration {
    let cadence = if active {
        IMAGES_PAR_SECONDE
    } else {
        IMAGES_PAR_SECONDE_INACTIVE
    };
    (debut + Duration::from_secs_f32(1.0 / cadence)).saturating_duration_since(maintenant)
}

/// La mesure affichée en bas à droite de la fenêtre, refaite chaque seconde.
pub struct Mesure {
    pub affichee: bool,
    /// Le début de la seconde en cours, ses images et leur temps de processeur.
    debut: Instant,
    images: u32,
    processeur: f32,
    /// Le résultat de la seconde précédente.
    resultat: Option<Resultat>,
}

/// Ce qu'a donné une seconde de mesure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resultat {
    pub images_par_seconde: f32,
    /// Temps de processeur moyen d'une image, en millisecondes.
    pub ms_par_image: f32,
}

impl Resultat {
    /// La part d'un cœur du processeur occupée par l'affichage, en %.
    pub fn part_d_un_coeur(&self) -> f32 {
        self.images_par_seconde * self.ms_par_image / 10.0
    }

    pub fn texte(&self) -> String {
        format!(
            "{:.0} images/s · {:.1} ms par image · {:.0} % d'un cœur",
            self.images_par_seconde,
            self.ms_par_image,
            self.part_d_un_coeur()
        )
        .replace('.', ",")
    }
}

impl Mesure {
    pub fn new(maintenant: Instant) -> Self {
        Self {
            affichee: false,
            debut: maintenant,
            images: 0,
            processeur: 0.0,
            resultat: None,
        }
    }

    /// Compte une image. `processeur` : le temps (en secondes) qu'a pris
    /// l'image précédente, d'après eframe (inconnu à la toute première).
    pub fn compter(&mut self, processeur: Option<f32>, maintenant: Instant) {
        if let Some(p) = processeur {
            self.images += 1;
            self.processeur += p;
        }
        let ecoule = maintenant.duration_since(self.debut).as_secs_f32();
        if ecoule >= 1.0 {
            self.resultat = (self.images > 0).then(|| Resultat {
                images_par_seconde: self.images as f32 / ecoule,
                ms_par_image: 1000.0 * self.processeur / self.images as f32,
            });
            self.debut = maintenant;
            self.images = 0;
            self.processeur = 0.0;
        }
    }

    /// La mesure en petit, en bas à droite, par-dessus tout le reste.
    pub fn dessiner(&self, ctx: &egui::Context) {
        if !self.affichee {
            return;
        }
        let texte = match self.resultat {
            Some(r) => r.texte(),
            None => "Mesure en cours…".to_owned(),
        };
        let peintre = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("mesure_processeur"),
        ));
        let coin = ctx.screen_rect().right_bottom() - egui::vec2(10.0, 8.0);
        let galet = peintre.layout_no_wrap(texte, egui::FontId::monospace(12.0), theme::JAUNE);
        let rect = egui::Align2::RIGHT_BOTTOM.anchor_size(coin, galet.size());
        peintre.rect_filled(rect.expand(4.0), 2.0, egui::Color32::from_black_alpha(200));
        peintre.galley(rect.min, galet, theme::JAUNE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_attend_ce_qui_reste_d_un_trentieme_de_seconde() {
        let debut = Instant::now();
        let trentieme = Duration::from_secs_f32(1.0 / 30.0);
        assert_eq!(attente(debut, debut, true), trentieme);
        let apres_10_ms = debut + Duration::from_millis(10);
        assert_eq!(
            attente(debut, apres_10_ms, true),
            trentieme - Duration::from_millis(10)
        );
        // Une image trop longue : on enchaîne tout de suite.
        assert_eq!(
            attente(debut, debut + Duration::from_millis(50), true),
            Duration::ZERO
        );
    }

    #[test]
    fn fenetre_inactive_on_attend_un_quinzieme_de_seconde() {
        let debut = Instant::now();
        assert_eq!(
            attente(debut, debut, false),
            Duration::from_secs_f32(1.0 / 15.0)
        );
    }

    #[test]
    fn la_mesure_fait_la_moyenne_de_chaque_seconde() {
        let debut = Instant::now();
        let mut mesure = Mesure::new(debut);
        mesure.compter(None, debut); // la toute première image : rien à compter
        for n in 1..=30 {
            mesure.compter(Some(0.004), debut + Duration::from_millis(n * 1000 / 30));
        }
        let r = mesure.resultat.expect("une seconde est passée");
        assert!((r.images_par_seconde - 30.0).abs() < 0.01);
        assert!((r.ms_par_image - 4.0).abs() < 0.01);
        assert!((r.part_d_un_coeur() - 12.0).abs() < 0.1);
        assert_eq!(r.texte(), "30 images/s · 4,0 ms par image · 12 % d'un cœur");
    }
}
