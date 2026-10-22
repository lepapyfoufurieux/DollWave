//! Le style « Ambiance », inspiré des visualiseurs de Windows Media Player
//! sous XP : des formes néon qui tournent lentement, gonflent sur les
//! graves, s'agitent sur les aigus, et laissent derrière elles une traînée
//! qui s'efface.

use std::collections::VecDeque;
use std::f32::consts::TAU;

use eframe::egui;

use crate::theme;

/// Nombre d'images gardées pour la traînée (environ 1/4 de seconde).
const TRAINEE: usize = 14;
/// Vitesse à laquelle les graves, médiums et aigus retombent (par seconde).
const CHUTE: f32 = 1.2;
/// Hauteur de barre en dessous de laquelle une bande compte pour rien :
/// sans ce seuil, le moindre bruit de fond ferait déjà gonfler les formes.
const SEUIL: f32 = 0.25;
/// Nombre de points de chaque forme.
const POINTS: usize = 160;

/// L'état des formes à un instant : de quoi les redessiner.
#[derive(Clone, Copy, PartialEq, Default)]
struct Instant {
    temps: f32,
    graves: f32,
    mediums: f32,
    aigus: f32,
}

pub struct Ambiance {
    /// Le temps de l'animation : il avance plus vite quand la musique est
    /// chargée, et s'arrête en pause.
    temps: f32,
    /// La force actuelle de chaque bande (entre 0 et 1).
    graves: f32,
    mediums: f32,
    aigus: f32,
    /// Les dernières images, de la plus ancienne à la plus récente.
    trainee: VecDeque<Instant>,
}

impl Ambiance {
    pub fn new() -> Self {
        Self {
            temps: 0.0,
            graves: 0.0,
            mediums: 0.0,
            aigus: 0.0,
            trainee: VecDeque::with_capacity(TRAINEE + 1),
        }
    }

    /// Vrai quand plus rien ne bouge : la musique s'est tue et la traînée
    /// a rejoint la forme immobile.
    pub fn au_repos(&self) -> bool {
        let silence = self.graves <= 0.0 && self.mediums <= 0.0 && self.aigus <= 0.0;
        silence && self.trainee.iter().all(|i| Some(i) == self.trainee.back())
    }

    /// Fait avancer l'animation d'une image. `barres` : la force de chaque
    /// bande de fréquences (entre 0 et 1), des graves aux aigus, toutes à 0
    /// en pause. `dt` : la durée de l'image, en secondes.
    pub fn avancer(&mut self, barres: &[f32], en_lecture: bool, dt: f32) {
        let n = barres.len();
        let force = |debut: usize, fin: usize| {
            let moyenne = barres[debut..fin].iter().sum::<f32>() / (fin - debut) as f32;
            ((moyenne - SEUIL) / (1.0 - SEUIL)).clamp(0.0, 1.0)
        };
        let cibles = [
            force(0, n / 5),
            force(n / 5, 2 * n / 3),
            force(2 * n / 3, n),
        ];
        // Comme les barres : on monte tout de suite, on redescend doucement.
        for (valeur, cible) in [&mut self.graves, &mut self.mediums, &mut self.aigus]
            .into_iter()
            .zip(cibles)
        {
            *valeur = cible.max(*valeur - CHUTE * dt);
        }
        if en_lecture {
            self.temps += dt * (0.35 + 1.2 * self.mediums + 0.8 * self.aigus);
        }
        self.trainee.push_back(Instant {
            temps: self.temps,
            graves: self.graves,
            mediums: self.mediums,
            aigus: self.aigus,
        });
        while self.trainee.len() > TRAINEE {
            self.trainee.pop_front();
        }
    }

    /// Dessine les formes dans `zone`, la traînée d'abord (de plus en plus
    /// visible), puis l'image actuelle avec sa lueur.
    pub fn dessiner(&self, peintre: &egui::Painter, zone: egui::Rect) {
        let peintre = peintre.with_clip_rect(zone);
        let nombre = self.trainee.len();
        for (i, instant) in self.trainee.iter().enumerate() {
            let derniere = i + 1 == nombre;
            // Les images anciennes sont pâles, la plus récente est pleine.
            let force = if derniere {
                1.0
            } else {
                0.45 * ((i + 1) as f32 / nombre as f32).powi(2)
            };
            for (points, couleur) in formes(instant, zone) {
                if derniere && theme::intensite() > 0.0 {
                    peintre.add(egui::Shape::closed_line(
                        points.clone(),
                        egui::Stroke::new(
                            5.0_f32,
                            theme::fondu(couleur, 0.18 * theme::intensite()),
                        ),
                    ));
                }
                let epaisseur = if derniere { 1.5_f32 } else { 1.0 };
                peintre.add(egui::Shape::closed_line(
                    points,
                    egui::Stroke::new(epaisseur, theme::fondu(couleur, force)),
                ));
            }
        }
    }
}

/// Les trois formes d'un instant, chacune avec sa couleur :
/// - une volute cyan qui balaie toute la largeur (elle s'ouvre sur les médiums) ;
/// - une rosace magenta à cinq pétales (elle gonfle sur les graves) ;
/// - un petit anneau jaune au centre (il bat sur les graves, frémit sur les aigus).
fn formes(i: &Instant, zone: egui::Rect) -> [(Vec<egui::Pos2>, egui::Color32); 3] {
    let c = zone.center();
    let ry = zone.height() / 2.0 - 6.0;
    let large = zone.width() / 2.0 - 8.0;
    // La rosace s'étire un peu en largeur, sans devenir un trait plat.
    let rx = large.min(ry * 2.5);
    let t = i.temps;
    let tour = |k: usize| TAU * k as f32 / POINTS as f32;

    let volute = (0..POINTS)
        .map(|k| {
            let u = tour(k);
            let x = large * (3.0 * u + 0.5 * t).sin();
            let y = ry * (0.3 + 0.6 * i.mediums) * (2.0 * u + 0.7 * t).sin()
                + ry * 0.08 * i.aigus * (17.0 * u + 5.0 * t).sin();
            c + egui::vec2(x, y)
        })
        .collect();

    let rosace = (0..POINTS)
        .map(|k| {
            let u = tour(k);
            let r = (0.45 + 0.25 * i.graves)
                * (1.0 + (0.15 + 0.25 * i.graves) * (5.0 * u + 1.3 * t).sin());
            let angle = u + 0.4 * t;
            c + egui::vec2(rx * r * angle.cos(), ry * r * angle.sin())
        })
        .collect();

    let anneau = (0..POINTS)
        .map(|k| {
            let u = tour(k);
            let r =
                ry * (0.12 + 0.25 * i.graves) * (1.0 + 0.2 * i.aigus * (8.0 * u - 2.0 * t).sin());
            c + egui::vec2(r * u.cos(), r * u.sin())
        })
        .collect();

    [
        (volute, theme::CYAN),
        (rosace, theme::MAGENTA),
        (anneau, theme::JAUNE),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: f32 = 1.0 / 60.0;

    #[test]
    fn les_graves_font_gonfler_les_graves_seulement() {
        let mut ambiance = Ambiance::new();
        let mut barres = [0.0; 32];
        barres[..6].fill(1.0);
        ambiance.avancer(&barres, true, IMAGE);
        assert!(ambiance.graves > 0.9);
        assert!(ambiance.mediums < 0.1 && ambiance.aigus < 0.1);
    }

    #[test]
    fn en_pause_tout_se_calme_puis_s_arrete() {
        let mut ambiance = Ambiance::new();
        let pleines = [1.0; 32];
        for _ in 0..30 {
            ambiance.avancer(&pleines, true, IMAGE);
        }
        assert!(!ambiance.au_repos());
        let temps = ambiance.temps;
        let mut images = 0;
        while !ambiance.au_repos() && images < 200 {
            ambiance.avancer(&[0.0; 32], false, IMAGE);
            images += 1;
        }
        // Moins d'une seconde pour retomber, plus le temps que la traînée rattrape.
        assert!(images <= 60 + TRAINEE, "{images} images pour se calmer");
        // En pause, les formes ne tournent plus.
        assert_eq!(ambiance.temps, temps);
    }

    #[test]
    fn les_formes_restent_dans_le_cadre() {
        let zone = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(600.0, 150.0));
        let fort = Instant {
            temps: 3.7,
            graves: 1.0,
            mediums: 1.0,
            aigus: 1.0,
        };
        for (points, _) in formes(&fort, zone) {
            assert!(points.iter().all(|p| zone.contains(*p)));
        }
    }
}
