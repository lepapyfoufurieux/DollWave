//! Le visualiseur : une image qui bouge avec la musique en cours de lecture.
//! Quatre styles au choix, dans un menu déroulant :
//! - les barres de fréquences (graves à gauche, aigus à droite) ;
//! - l'oscilloscope (la vibration du son, en direct) ;
//! - la forme d'onde (le dessin des 30 secondes autour de l'endroit écouté) ;
//! - l'ambiance (des formes qui tournent avec la musique, façon Windows Media Player XP).

mod ambiance;

use std::sync::{Arc, OnceLock, Weak};
use std::thread;

use eframe::egui;
use rustfft::{num_complex::Complex, Fft, FftPlanner};

use crate::bibliotheque::memoriser;
use crate::morceau::Morceau;
use crate::systeme::fichier_de_reglage;
use crate::theme;
use ambiance::Ambiance;

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
/// Finesse du résumé de la forme d'onde : nombre de tranches par seconde.
const TRANCHES_PAR_SECONDE: u32 = 100;
/// Durée (en secondes) montrée par la forme d'onde : la moitié avant
/// l'endroit écouté, la moitié après. Elle défile pendant la lecture.
const FENETRE: f32 = 30.0;
/// Hauteur de la zone de dessin, en points.
const HAUTEUR: f32 = 150.0;
/// La marge entre le cadre néon et ce qu'il contient.
const MARGE: f32 = 12.0;
/// Le petit fichier où l'on mémorise le style choisi
/// (%APPDATA%\lecteur_mp3\visualiseur.txt).
const REGLAGE_STYLE: &str = "visualiseur.txt";

/// Le résumé de la forme d'onde : le plus bas et le plus haut de chaque tranche.
type Apercu = Vec<(f32, f32)>;

/// Les styles de visualiseur proposés dans le menu.
#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    Barres,
    Oscilloscope,
    FormeOnde,
    Ambiance,
}

impl Style {
    const TOUS: [Style; 4] = [
        Style::Barres,
        Style::Oscilloscope,
        Style::FormeOnde,
        Style::Ambiance,
    ];

    /// Le titre du cadre, en néon, pour ce style.
    fn titre(self) -> &'static str {
        match self {
            Style::Barres => "SPECTRE",
            Style::Oscilloscope => "OSCILLOSCOPE",
            Style::FormeOnde => "FORME D'ONDE",
            Style::Ambiance => "AMBIANCE",
        }
    }

    fn nom(self) -> &'static str {
        match self {
            Style::Barres => "Barres de fréquences",
            Style::Oscilloscope => "Oscilloscope",
            Style::FormeOnde => "Forme d'onde",
            Style::Ambiance => "Ambiance",
        }
    }

    /// Le mot écrit dans le fichier de réglage pour ce style.
    fn code(self) -> &'static str {
        match self {
            Style::Barres => "barres",
            Style::Oscilloscope => "oscilloscope",
            Style::FormeOnde => "onde",
            Style::Ambiance => "ambiance",
        }
    }

    /// Retrouve un style à partir du mot du fichier de réglage.
    fn depuis_code(texte: &str) -> Option<Style> {
        Style::TOUS.into_iter().find(|s| s.code() == texte.trim())
    }

    /// Le style mémorisé au dernier lancement, ou les barres par défaut.
    fn memorise() -> Style {
        fichier_de_reglage(REGLAGE_STYLE)
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|texte| Style::depuis_code(&texte))
            .unwrap_or(Style::Barres)
    }
}

pub struct Visualiseur {
    style: Style,
    /// L'outil qui découpe le son en fréquences (préparé une seule fois),
    /// avec sa fenêtre de Hann et ses tableaux de travail, réutilisés à
    /// chaque image au lieu d'être refaits.
    fft: Arc<dyn Fft<f32>>,
    fenetre: Vec<f32>,
    spectre: Vec<Complex<f32>>,
    brouillon: Vec<Complex<f32>>,
    /// Hauteur actuelle de chaque barre (entre 0 et 1).
    barres: [f32; NB_BARRES],
    /// Les formes du style « Ambiance ».
    ambiance: Ambiance,
    /// Le résumé de la forme d'onde, calculé en arrière-plan une fois par
    /// morceau (vide tant que le calcul n'est pas fini). On ne garde pas le
    /// morceau lui-même (`Weak`) : une fois fini, il peut libérer sa mémoire.
    apercu: Option<(Weak<Morceau>, Arc<OnceLock<Apercu>>)>,
}

impl Visualiseur {
    pub fn new() -> Self {
        Self {
            style: Style::memorise(),
            fft: FftPlanner::new().plan_fft_forward(TAILLE_FFT),
            fenetre: (0..TAILLE_FFT)
                .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / TAILLE_FFT as f32).cos())
                .collect(),
            spectre: Vec::with_capacity(TAILLE_FFT),
            brouillon: Vec::new(),
            barres: [0.0; NB_BARRES],
            ambiance: Ambiance::new(),
            apercu: None,
        }
    }

    /// Vrai quand plus rien ne bouge (les barres sont retombées, les formes
    /// de l'ambiance sont immobiles) : inutile alors de redessiner à chaque image.
    pub fn au_repos(&self) -> bool {
        match self.style {
            Style::Barres => self.barres.iter().all(|&b| b <= 0.0),
            Style::Ambiance => self.ambiance.au_repos(),
            Style::Oscilloscope | Style::FormeOnde => true,
        }
    }

    /// Dessine le cadre du visualiseur : son titre (« SPECTRE »), le menu de
    /// choix du style à droite, puis l'image qui bouge.
    /// `position` : où en est la lecture (en secondes). `en_lecture` : faux si
    /// la musique est en pause ou arrêtée (les barres retombent alors à zéro).
    pub fn afficher(
        &mut self,
        ui: &mut egui::Ui,
        morceau: Option<&Arc<Morceau>>,
        position: f32,
        en_lecture: bool,
    ) {
        // Le cadre néon, magenta, aux angles coupés en haut à droite et en bas à gauche.
        let cadre = egui::Rect::from_min_size(
            ui.available_rect_before_wrap().min,
            egui::vec2(ui.available_width(), HAUTEUR + 2.0 * MARGE + 32.0),
        );
        ui.allocate_rect(cadre, egui::Sense::hover());
        theme::cadre(
            ui.painter(),
            cadre,
            theme::Coins::autre_diagonale(16.0),
            theme::FOND,
            egui::Stroke::new(1.0_f32, theme::attenue(theme::MAGENTA, 110)),
            0.0,
        );
        let mut dedans = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(cadre.shrink(MARGE))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut dedans;

        let avant = self.style;
        ui.horizontal(|ui| {
            ui.set_min_height(24.0);
            let (place, _) = ui.allocate_exact_size(egui::vec2(140.0, 24.0), egui::Sense::hover());
            theme::texte_lumineux(
                ui.painter(),
                place.left_center(),
                egui::Align2::LEFT_CENTER,
                self.style.titre(),
                crate::police::titres(ui.ctx(), 12.0),
                2.0,
                theme::MAGENTA,
                theme::MAGENTA,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("style_du_visualiseur")
                    .selected_text(self.style.nom().to_uppercase())
                    .show_ui(ui, |ui| {
                        for style in Style::TOUS {
                            ui.selectable_value(&mut self.style, style, style.nom());
                        }
                    });
            });
        });
        if self.style != avant {
            memoriser(REGLAGE_STYLE, self.style.code());
        }
        ui.add_space(6.0);

        let taille = egui::vec2(ui.available_width(), HAUTEUR);
        let (reponse, peintre) = ui.allocate_painter(taille, egui::Sense::hover());
        let zone = reponse.rect;
        peintre.hline(
            zone.x_range(),
            zone.bottom(),
            egui::Stroke::new(1.0_f32, theme::attenue(theme::CYAN, 110)),
        );
        let couleur = theme::CYAN;
        let dt = ui.input(|i| i.stable_dt);
        if morceau.is_none() {
            self.apercu = None; // plus de morceau (arrêt, liste vidée)
        }

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
                if let Some(apercu) = morceau.and_then(|m| self.apercu_de(m)) {
                    let grise = theme::TEXTE_DISCRET;
                    let curseur = theme::JAUNE;
                    dessiner_forme_onde(&peintre, zone, apercu, position, couleur, grise, curseur);
                }
            }
            Style::Ambiance => {
                let forces = match morceau {
                    Some(m) if en_lecture => self.hauteurs_des_barres(m, position),
                    _ => [0.0; NB_BARRES],
                };
                self.ambiance.avancer(&forces, en_lecture, dt);
                self.ambiance.dessiner(&peintre, zone);
            }
        }
        // Les lignes d'écran, plus marquées ici que sur le reste de la fenêtre.
        theme::lignes_d_ecran(&peintre, zone, 3.0, 71.0);
    }

    /// Le résumé de la forme d'onde du morceau. La première fois, on lance son
    /// calcul en arrière-plan (il faut décoder tout le morceau) : en attendant,
    /// il n'y a rien à dessiner.
    /// Si on change de morceau avant la fin, le calcul de l'ancien s'arrête.
    /// (Sert aussi à la barre de progression, qui dessine tout le morceau.)
    pub fn apercu_de(&mut self, morceau: &Arc<Morceau>) -> Option<&[(f32, f32)]> {
        let deja_lance =
            matches!(&self.apercu, Some((m, _)) if std::ptr::eq(m.as_ptr(), Arc::as_ptr(morceau)));
        if !deja_lance {
            let resultat = Arc::new(OnceLock::new());
            let (m, r) = (Arc::downgrade(morceau), resultat.clone());
            thread::spawn(move || {
                if let Some(apercu) = Morceau::apercu_tant_qu_utile(&m, TRANCHES_PAR_SECONDE) {
                    let _ = r.set(apercu);
                }
            });
            self.apercu = Some((Arc::downgrade(morceau), resultat));
        }
        self.apercu.as_ref().unwrap().1.get().map(Vec::as_slice)
    }

    /// Analyse le son à partir de `position` et renvoie la hauteur de chaque
    /// barre (entre 0 et 1), des graves aux aigus.
    fn hauteurs_des_barres(&mut self, morceau: &Morceau, position: f32) -> [f32; NB_BARRES] {
        let echantillons = morceau.extrait_mono(position, TAILLE_FFT);

        // On adoucit les deux bouts de l'extrait (fenêtre de Hann) : sinon la
        // coupure brutale ajouterait des fréquences qui n'existent pas.
        self.spectre.clear();
        self.spectre.extend(
            echantillons
                .iter()
                .zip(&self.fenetre)
                .map(|(&e, &f)| Complex::new(e * f, 0.0)),
        );
        self.brouillon
            .resize(self.fft.get_inplace_scratch_len(), Complex::default());
        self.fft
            .process_with_scratch(&mut self.spectre, &mut self.brouillon);
        let spectre = &self.spectre;

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
    // Chaque barre : un dégradé cyan (en bas) → magenta (en haut, à pleine
    // hauteur), une lueur magenta, et un petit chapeau jaune.
    let largeur = zone.width() / barres.len() as f32;
    let lueur = theme::intensite();
    for (i, &hauteur) in barres.iter().enumerate() {
        if hauteur <= 0.0 {
            continue;
        }
        let gauche = zone.left() + i as f32 * largeur;
        let haut = zone.bottom() - hauteur * (zone.height() - 6.0);
        let barre = egui::Rect::from_min_max(
            egui::pos2(gauche + 1.5, haut),
            egui::pos2(gauche + largeur - 1.5, zone.bottom()),
        );
        if lueur > 0.0 {
            peintre.rect_filled(
                barre.expand(1.5 * lueur),
                0.0,
                theme::attenue(theme::MAGENTA, (40.0 * lueur.min(1.5)) as u8),
            );
        }
        let sommet = theme::melange(couleur, theme::MAGENTA, hauteur);
        theme::degrade(
            peintre,
            &theme::contour(barre, theme::Coins::AUCUN),
            couleur,
            sommet,
            -egui::Vec2::Y,
        );
        let chapeau = egui::Rect::from_min_max(
            egui::pos2(barre.left(), haut - 6.0),
            egui::pos2(barre.right(), haut - 3.0),
        );
        peintre.rect_filled(chapeau, 0.0, theme::JAUNE);
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
    // Un trait large et transparent dessous fait la lueur.
    let points: Vec<egui::Pos2> = points;
    if theme::intensite() > 0.0 {
        peintre.add(egui::Shape::line(
            points.clone(),
            egui::Stroke::new(5.0_f32, theme::fondu(couleur, 0.18 * theme::intensite())),
        ));
    }
    peintre.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5_f32, couleur),
    ));
}

/// Dessine les 30 secondes de musique autour de `position` (en secondes), qui reste
/// au milieu : la partie déjà jouée est en couleur, le reste en gris.
fn dessiner_forme_onde(
    peintre: &egui::Painter,
    zone: egui::Rect,
    apercu: &[(f32, f32)],
    position: f32,
    couleur: egui::Color32,
    grise: egui::Color32,
    curseur: egui::Color32,
) {
    let milieu = zone.center().y;
    let demi_hauteur = zone.height() / 2.0;
    let colonnes = zone.width().max(1.0) as usize;
    // Durée couverte par une colonne de pixels.
    let par_colonne = FENETRE / colonnes as f32;
    let debut = position - FENETRE / 2.0;
    for c in 0..colonnes {
        // Les tranches du résumé qui tombent dans cette colonne de pixels.
        let temps = debut + c as f32 * par_colonne;
        let premiere = (temps * TRANCHES_PAR_SECONDE as f32).floor() as isize;
        let derniere = ((temps + par_colonne) * TRANCHES_PAR_SECONDE as f32).floor() as isize;
        let premiere = premiere.max(0) as usize;
        let derniere = (derniere.max(0) as usize)
            .max(premiere + 1)
            .min(apercu.len());
        // Avant le début ou après la fin du morceau : rien à dessiner.
        let Some(tranches) = apercu.get(premiere..derniere).filter(|t| !t.is_empty()) else {
            continue;
        };
        let bas = tranches.iter().map(|t| t.0).fold(0.0, f32::min);
        let haut = tranches.iter().map(|t| t.1).fold(0.0, f32::max);

        let x = zone.left() + c as f32 + 0.5;
        let teinte = if temps < position { couleur } else { grise };
        peintre.line_segment(
            [
                egui::pos2(x, milieu - haut * demi_hauteur),
                egui::pos2(x, milieu - bas * demi_hauteur + 1.0),
            ],
            egui::Stroke::new(1.0_f32, teinte),
        );
    }
    let x_lecture = zone.center().x;
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
    fn chaque_style_se_relit_depuis_son_mot() {
        for style in Style::TOUS {
            assert!(Style::depuis_code(style.code()) == Some(style));
        }
        assert!(Style::depuis_code("oscilloscope\r\n") == Some(Style::Oscilloscope));
        assert!(Style::depuis_code("n'importe quoi").is_none());
    }

    #[test]
    fn un_son_a_1000_hz_allume_la_bonne_barre() {
        // Un son pur à 1000 Hz, au volume maximum, pendant 1 seconde.
        let morceau = Morceau::sinus(1000.0, 44_100, 1.0);
        let mut visualiseur = Visualiseur::new();
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

    /// En pause, les barres retombent puis le visualiseur se dit « au repos »
    /// (l'appli arrête alors de redessiner à chaque image).
    #[test]
    fn en_pause_les_barres_retombent_puis_tout_s_arrete() {
        let morceau = Arc::new(Morceau::sinus(1000.0, 44_100, 1.0));
        let mut visualiseur = Visualiseur::new();
        visualiseur.style = Style::Barres;
        let ctx = egui::Context::default();
        // Les polices néon (le titre « SPECTRE », la phrase en cours) en ont besoin.
        crate::police::installer(&ctx);
        let image = |visualiseur: &mut Visualiseur, en_lecture: bool| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    visualiseur.afficher(ui, Some(&morceau), 0.0, en_lecture);
                });
            });
        };
        image(&mut visualiseur, true);
        assert!(!visualiseur.au_repos());
        // Les barres mettent moins d'une seconde à retomber (60 images).
        let mut images = 0;
        while !visualiseur.au_repos() && images < 120 {
            image(&mut visualiseur, false);
            images += 1;
        }
        assert!(images <= 60, "{images} images pour retomber");
    }

    /// L'ambiance bouge pendant la lecture, puis s'immobilise en pause.
    #[test]
    fn l_ambiance_bouge_puis_s_immobilise_en_pause() {
        let morceau = Arc::new(Morceau::sinus(60.0, 44_100, 1.0));
        let mut visualiseur = Visualiseur::new();
        visualiseur.style = Style::Ambiance;
        let ctx = egui::Context::default();
        crate::police::installer(&ctx);
        let image = |visualiseur: &mut Visualiseur, en_lecture: bool| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    visualiseur.afficher(ui, Some(&morceau), 0.0, en_lecture);
                });
            });
        };
        image(&mut visualiseur, true);
        assert!(!visualiseur.au_repos());
        let mut images = 0;
        while !visualiseur.au_repos() && images < 200 {
            image(&mut visualiseur, false);
            images += 1;
        }
        assert!(
            visualiseur.au_repos(),
            "toujours en mouvement après {images} images"
        );
    }

    /// La forme d'onde ne garde pas le morceau en mémoire : une fois le
    /// calcul fini, seul le lecteur le tient encore.
    #[test]
    fn la_forme_d_onde_ne_garde_pas_le_morceau() {
        let morceau = Arc::new(Morceau::sinus(440.0, 8000, 2.0));
        let mut visualiseur = Visualiseur::new();
        visualiseur.style = Style::FormeOnde;
        let ctx = egui::Context::default();
        // Les polices néon (le titre « SPECTRE », la phrase en cours) en ont besoin.
        crate::police::installer(&ctx);
        let image = |visualiseur: &mut Visualiseur, morceau: Option<&Arc<Morceau>>| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    visualiseur.afficher(ui, morceau, 0.0, true);
                });
            });
        };
        image(&mut visualiseur, Some(&morceau));
        let debut = std::time::Instant::now();
        while visualiseur.apercu.as_ref().unwrap().1.get().is_none()
            && debut.elapsed() < std::time::Duration::from_secs(2)
        {
            thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(visualiseur.apercu.as_ref().unwrap().1.get().is_some());
        // Le temps que le fil de calcul lâche le morceau.
        thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(Arc::strong_count(&morceau), 1);
        image(&mut visualiseur, None);
        assert!(visualiseur.apercu.is_none());
    }
}
