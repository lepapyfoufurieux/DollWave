//! Le visualiseur : une image qui bouge avec la musique en cours de lecture.
//! Trois styles au choix, dans un menu déroulant :
//! - les barres de fréquences (graves à gauche, aigus à droite) ;
//! - l'oscilloscope (la vibration du son, en direct) ;
//! - la forme d'onde (le dessin de tout le morceau).

use std::sync::Arc;

use eframe::egui;
use rustfft::{num_complex::Complex, Fft, FftPlanner};

use crate::platine::Morceau;

/// Nombre d'échantillons analysés pour les barres. Plus c'est grand, plus les
/// graves sont précis, mais plus les barres réagissent lentement.
const TAILLE_FFT: usize = 2048;
/// Nombre de barres affichées.
const NB_BARRES: usize = 32;
/// Fréquences (en Hz) de la première et de la dernière barre.
const FREQUENCE_MIN: f32 = 30.0;
const FREQUENCE_MAX: f32 = 16_000.0;
/// Volume (en décibels) d'une barre vide et d'une barre pleine.
const DB_MIN: f32 = -65.0;
const DB_MAX: f32 = -5.0;
/// Vitesse à laquelle les barres retombent (hauteur totale par seconde).
const CHUTE: f32 = 1.5;
/// Nombre d'échantillons dessinés par l'oscilloscope (environ 1/40e de seconde).
const TAILLE_OSCILLO: usize = 1024;
/// Nombre de colonnes du résumé de la forme d'onde.
const COLONNES_APERCU: usize = 1000;
/// Hauteur de la zone de dessin, en points.
const HAUTEUR: f32 = 150.0;

/// Les styles de visualiseur proposés dans le menu.
#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    Barres,
    Oscilloscope,
    FormeOnde,
}

impl Style {
    const TOUS: [Style; 3] = [Style::Barres, Style::Oscilloscope, Style::FormeOnde];

    fn nom(self) -> &'static str {
        match self {
            Style::Barres => "Barres de fréquences",
            Style::Oscilloscope => "Oscilloscope",
            Style::FormeOnde => "Forme d'onde",
        }
    }
}

pub struct Visualiseur {
    style: Style,
    /// L'outil qui découpe le son en fréquences (préparé une seule fois).
    fft: Arc<dyn Fft<f32>>,
    /// Hauteur actuelle de chaque barre (entre 0 et 1).
    barres: [f32; NB_BARRES],
    /// Le résumé de la forme d'onde, calculé une fois par morceau.
    apercu: Option<(Arc<Morceau>, Vec<(f32, f32)>)>,
}

impl Visualiseur {
    pub fn new() -> Self {
        Self {
            style: Style::Barres,
            fft: FftPlanner::new().plan_fft_forward(TAILLE_FFT),
            barres: [0.0; NB_BARRES],
            apercu: None,
        }
    }

    /// Dessine le menu de choix puis le visualiseur.
    /// `position` : où en est la lecture (en secondes). `en_lecture` : faux si
    /// la musique est en pause ou arrêtée (les barres retombent alors à zéro).
    pub fn afficher(
        &mut self,
        ui: &mut egui::Ui,
        morceau: Option<&Arc<Morceau>>,
        position: f32,
        en_lecture: bool,
    ) {
        egui::ComboBox::from_label("Visualiseur")
            .selected_text(self.style.nom())
            .show_ui(ui, |ui| {
                for style in Style::TOUS {
                    ui.selectable_value(&mut self.style, style, style.nom());
                }
            });

        let taille = egui::vec2(ui.available_width(), HAUTEUR);
        let (reponse, peintre) = ui.allocate_painter(taille, egui::Sense::hover());
        let zone = reponse.rect;
        peintre.rect_filled(zone, 4.0, ui.visuals().extreme_bg_color);
        let couleur = ui.visuals().selection.bg_fill;
        let dt = ui.input(|i| i.stable_dt);

        match self.style {
            Style::Barres => {
                let cibles = match morceau {
                    Some(m) if en_lecture => self.hauteurs_des_barres(m, position),
                    _ => [0.0; NB_BARRES],
                };
                // Une barre monte tout de suite, mais redescend doucement.
                for (barre, cible) in self.barres.iter_mut().zip(cibles) {
                    *barre = cible.max(*barre - CHUTE * dt);
                }
                dessiner_barres(&peintre, zone, &self.barres, couleur);
            }
            Style::Oscilloscope => {
                let echantillons = match morceau {
                    Some(m) if en_lecture => m.extrait_mono(position, TAILLE_OSCILLO),
                    _ => vec![0.0; TAILLE_OSCILLO],
                };
                dessiner_oscilloscope(&peintre, zone, &echantillons, couleur);
            }
            Style::FormeOnde => {
                if let Some(morceau) = morceau {
                    let duree = morceau.duree().as_secs_f32();
                    let avancement = if duree > 0.0 { position / duree } else { 0.0 };
                    let apercu = self.apercu_de(morceau);
                    let grise = ui.visuals().weak_text_color();
                    let curseur = ui.visuals().strong_text_color();
                    dessiner_forme_onde(
                        &peintre, zone, apercu, avancement, couleur, grise, curseur,
                    );
                }
            }
        }
    }

    /// Le résumé de la forme d'onde du morceau, calculé seulement la première fois.
    fn apercu_de(&mut self, morceau: &Arc<Morceau>) -> &[(f32, f32)] {
        let deja_calcule = matches!(&self.apercu, Some((m, _)) if Arc::ptr_eq(m, morceau));
        if !deja_calcule {
            self.apercu = Some((morceau.clone(), morceau.apercu(COLONNES_APERCU)));
        }
        &self.apercu.as_ref().unwrap().1
    }

    /// Analyse le son à partir de `position` et renvoie la hauteur de chaque
    /// barre (entre 0 et 1), des graves aux aigus.
    fn hauteurs_des_barres(&self, morceau: &Morceau, position: f32) -> [f32; NB_BARRES] {
        let echantillons = morceau.extrait_mono(position, TAILLE_FFT);

        // On adoucit les deux bouts de l'extrait (fenêtre de Hann) : sinon la
        // coupure brutale ajouterait des fréquences qui n'existent pas.
        let mut spectre: Vec<Complex<f32>> = echantillons
            .iter()
            .enumerate()
            .map(|(i, &e)| {
                let fenetre =
                    0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / TAILLE_FFT as f32).cos();
                Complex::new(e * fenetre, 0.0)
            })
            .collect();
        self.fft.process(&mut spectre);

        // Chaque case du spectre couvre une petite plage de fréquences.
        let largeur_case = morceau.frequence() as f32 / TAILLE_FFT as f32;
        let nb_cases = TAILLE_FFT / 2;
        // Pour qu'une sinusoïde au maximum du volume donne 0 dB.
        let normalisation = 4.0 / TAILLE_FFT as f32;

        let mut hauteurs = [0.0; NB_BARRES];
        for (b, hauteur) in hauteurs.iter_mut().enumerate() {
            // Les barres sont réparties comme les touches d'un piano : chacune
            // couvre la même proportion de fréquences (sinon les aigus prendraient
            // presque toute la place).
            let debut = frequence_de_barre(b) / largeur_case;
            let fin = frequence_de_barre(b + 1) / largeur_case;
            let debut = (debut as usize).min(nb_cases - 1);
            let fin = (fin as usize).clamp(debut + 1, nb_cases);
            let force = spectre[debut..fin]
                .iter()
                .map(|c| c.norm() * normalisation)
                .fold(0.0, f32::max);
            let db = 20.0 * force.max(1e-9).log10();
            *hauteur = ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0);
        }
        hauteurs
    }
}

/// La fréquence où commence la barre numéro `b`.
fn frequence_de_barre(b: usize) -> f32 {
    FREQUENCE_MIN * (FREQUENCE_MAX / FREQUENCE_MIN).powf(b as f32 / NB_BARRES as f32)
}

fn dessiner_barres(
    peintre: &egui::Painter,
    zone: egui::Rect,
    barres: &[f32],
    couleur: egui::Color32,
) {
    let largeur = zone.width() / barres.len() as f32;
    for (i, &hauteur) in barres.iter().enumerate() {
        let gauche = zone.left() + i as f32 * largeur;
        let barre = egui::Rect::from_min_max(
            egui::pos2(gauche + 1.0, zone.bottom() - hauteur * zone.height()),
            egui::pos2(gauche + largeur - 1.0, zone.bottom()),
        );
        peintre.rect_filled(barre, 1.0, couleur);
    }
}

fn dessiner_oscilloscope(
    peintre: &egui::Painter,
    zone: egui::Rect,
    echantillons: &[f32],
    couleur: egui::Color32,
) {
    let dernier = (echantillons.len() - 1).max(1) as f32;
    let points = echantillons
        .iter()
        .enumerate()
        .map(|(i, &e)| {
            egui::pos2(
                zone.left() + zone.width() * i as f32 / dernier,
                zone.center().y - e.clamp(-1.0, 1.0) * zone.height() / 2.0,
            )
        })
        .collect();
    peintre.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5_f32, couleur),
    ));
}

/// Dessine tout le morceau ; la partie déjà jouée (avant `avancement`, entre
/// 0 et 1) est en couleur, le reste en gris, avec un trait à l'endroit joué.
fn dessiner_forme_onde(
    peintre: &egui::Painter,
    zone: egui::Rect,
    apercu: &[(f32, f32)],
    avancement: f32,
    couleur: egui::Color32,
    grise: egui::Color32,
    curseur: egui::Color32,
) {
    let milieu = zone.center().y;
    let demi_hauteur = zone.height() / 2.0;
    let x_lecture = zone.left() + avancement.clamp(0.0, 1.0) * zone.width();
    let colonnes = zone.width().max(1.0) as usize;
    for c in 0..colonnes {
        // Chaque colonne de pixels montre la tranche du résumé qui lui correspond.
        let (bas, haut) = apercu[c * apercu.len() / colonnes];
        let x = zone.left() + c as f32 + 0.5;
        let teinte = if x < x_lecture { couleur } else { grise };
        peintre.line_segment(
            [
                egui::pos2(x, milieu - haut * demi_hauteur),
                egui::pos2(x, milieu - bas * demi_hauteur + 1.0),
            ],
            egui::Stroke::new(1.0_f32, teinte),
        );
    }
    peintre.line_segment(
        [
            egui::pos2(x_lecture, zone.top()),
            egui::pos2(x_lecture, zone.bottom()),
        ],
        egui::Stroke::new(2.0_f32, curseur),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_son_a_1000_hz_allume_la_bonne_barre() {
        // Un son pur à 1000 Hz, au volume maximum, pendant 1 seconde.
        let morceau = Morceau::sinus(1000.0, 44_100, 1.0);
        let visualiseur = Visualiseur::new();
        let hauteurs = visualiseur.hauteurs_des_barres(&morceau, 0.0);

        // La barre qui contient 1000 Hz est pleine…
        let bonne = (0..NB_BARRES)
            .find(|&b| frequence_de_barre(b + 1) > 1000.0)
            .unwrap();
        assert!(hauteurs[bonne] > 0.9, "hauteurs {hauteurs:?}");
        // …et les barres éloignées (au-delà des voisines) restent presque vides.
        for (b, &hauteur) in hauteurs.iter().enumerate() {
            if b.abs_diff(bonne) > 1 {
                assert!(hauteur < 0.2, "barre {b} : hauteurs {hauteurs:?}");
            }
        }
    }
}
