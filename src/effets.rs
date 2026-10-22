//! Les effets du thème néon (étape 8) : lignes d'écran façon vieux moniteur,
//! écran cathodique de vieille télé, et lueurs qui respirent pendant la
//! lecture (en pause, tout se fige).
//!
//! L'écran se choisit dans ⚙ Options (Aucun, Lignes d'écran ou Écran
//! cathodique) ; le choix est mémorisé dans le dossier de réglages
//! (lignes_ecran.txt : « non », « oui » ou « cathodique »).

use std::cell::Cell;

use eframe::egui::{self, epaint, Color32, Pos2, Rect, Stroke};

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
/// Écran cathodique : des lignes de balayage plus serrées et plus marquées.
const ECART_TUBE: f32 = 3.0;
const OPACITE_TUBE: f32 = 64.0;
/// Le bombé du tube : au bord, les lignes du haut et du bas se courbent
/// de ce nombre de points vers le milieu.
const BOMBE: f32 = 5.0;
/// Les coins arrondis noirs du tube (rayon, en points).
const RAYON_COINS: f32 = 22.0;
/// L'ombre des bords (vignette) : opacité tout au bord de l'écran.
const OPACITE_VIGNETTE: f32 = 150.0;
/// Le scintillement : opacité maximale du voile sombre qui tremble.
const OPACITE_SCINTILLEMENT: f32 = 10.0;
/// La bande lumineuse qui descend : hauteur, durée d'un passage, opacité.
const HAUTEUR_BANDE: f32 = 90.0;
const PERIODE_BANDE: f32 = 7.0;
const OPACITE_BANDE: f32 = 9.0;

/// Ce qui passe par-dessus la fenêtre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ecran {
    /// Rien : l'image toute nette.
    Aucun,
    /// De fines lignes sombres (sauf sur la pochette, pour les paroles).
    Lignes,
    /// Vieille télé : lignes de balayage, bords bombés, scintillement,
    /// partout, paroles comprises.
    Cathodique,
}

pub struct Effets {
    /// L'effet d'écran choisi.
    pub ecran: Ecran,
    /// Où en est la respiration des lueurs (en secondes).
    phase: f32,
    /// Le temps écoulé pendant la lecture (en secondes), pour faire
    /// scintiller le tube ; il ne bouge pas en pause.
    temps: f32,
    /// Une zone où les lignes de la fenêtre ne passent pas (la pochette :
    /// elle a ses propres lignes, sous les paroles). Remise à zéro à chaque image.
    sans_lignes: Cell<Option<Rect>>,
}

impl Effets {
    /// Les effets tels qu'on les a laissés au dernier lancement.
    pub fn charger() -> Self {
        let lire = |nom| fichier_de_reglage(nom).and_then(|f| std::fs::read_to_string(f).ok());
        let effets = Self {
            ecran: ecran_depuis(lire(REGLAGE_LIGNES).as_deref()),
            phase: 0.0,
            temps: 0.0,
            sans_lignes: Cell::new(None),
        };
        effets.appliquer();
        effets
    }

    /// Transmet le réglage au thème, qui dessine les lignes.
    fn appliquer(&self) {
        theme::fixer_lignes(self.ecran != Ecran::Aucun);
    }

    /// L'écran cathodique est-il choisi ? (Ses effets passent alors aussi
    /// sur la pochette et les paroles.)
    pub fn cathodique(&self) -> bool {
        self.ecran == Ecran::Cathodique
    }

    /// À chaque image : la respiration des lueurs avance seulement pendant
    /// la lecture. Sinon, elles restent allumées en plein, sans bouger.
    pub fn avancer(&mut self, ecoule: f32, en_lecture: bool) {
        if en_lecture {
            self.phase = (self.phase + ecoule) % PERIODE;
            // Remis à zéro de temps en temps pour garder des calculs précis.
            self.temps = (self.temps + ecoule) % (PERIODE_BANDE * 1000.0);
            theme::fixer_pulsation(respiration(self.phase));
        } else {
            theme::fixer_pulsation(1.0);
        }
    }

    /// Les lignes de la fenêtre ne passeront pas sur `zone` à cette image.
    pub fn proteger(&self, zone: Rect) {
        self.sans_lignes.set(Some(zone));
    }

    /// L'effet d'écran par-dessus toute la fenêtre (mais sous les menus
    /// et les fenêtres, qui restent bien nets). Les simples lignes évitent
    /// la zone protégée ; l'écran cathodique passe partout.
    /// À appeler après les panneaux.
    pub fn dessiner(&self, ctx: &egui::Context) {
        let calque =
            egui::LayerId::new(egui::Order::PanelResizeLine, egui::Id::new("lignes_ecran"));
        let peintre = ctx.layer_painter(calque);
        let ecran = ctx.screen_rect();
        let protegee = self.sans_lignes.take();
        match self.ecran {
            Ecran::Aucun => {}
            Ecran::Lignes => {
                for zone in autour(ecran, protegee) {
                    theme::lignes_d_ecran(&peintre, zone, ECART_FENETRE, OPACITE_FENETRE);
                }
            }
            Ecran::Cathodique => dessiner_tube(&peintre, ecran, self.temps),
        }
    }

    /// Le réglage des effets, dans le menu ⚙ Options.
    pub fn menu(&mut self, ui: &mut egui::Ui) {
        ui.label("Écran :");
        let choix = [
            (Ecran::Aucun, "Aucun", "L'image toute nette"),
            (
                Ecran::Lignes,
                "Lignes d'écran",
                "De fines lignes sombres, comme sur un vieux moniteur",
            ),
            (
                Ecran::Cathodique,
                "Écran cathodique",
                "Une vieille télé à tube : lignes de balayage, bords bombés, \
                 léger scintillement pendant la lecture",
            ),
        ];
        for (valeur, nom, aide) in choix {
            if ui
                .radio_value(&mut self.ecran, valeur, nom)
                .on_hover_text(aide)
                .changed()
            {
                memoriser(REGLAGE_LIGNES, texte_ecran(self.ecran));
                self.appliquer();
            }
        }
    }
}

/// L'écran cathodique sur `ecran`, au moment `temps` de la lecture :
/// scintillement, bande lumineuse qui descend, lignes de balayage bombées,
/// bords assombris et coins arrondis noirs du tube.
fn dessiner_tube(peintre: &egui::Painter, ecran: Rect, temps: f32) {
    // Le voile sombre qui tremble très légèrement.
    let voile = (OPACITE_SCINTILLEMENT * scintillement(temps)).round() as u8;
    peintre.rect_filled(ecran, 0.0, Color32::from_black_alpha(voile));

    // La bande un peu plus claire qui descend lentement.
    let haut = position_bande(temps, ecran.height()) + ecran.top();
    let bande = Rect::from_min_max(
        egui::pos2(ecran.left(), haut),
        egui::pos2(ecran.right(), haut + HAUTEUR_BANDE),
    );
    let clair = Color32::from_white_alpha(OPACITE_BANDE as u8);
    let mut maillage = epaint::Mesh::default();
    for (i, (y, couleur)) in [
        (bande.top(), Color32::TRANSPARENT),
        (bande.center().y, clair),
        (bande.bottom(), Color32::TRANSPARENT),
    ]
    .into_iter()
    .enumerate()
    {
        maillage.colored_vertex(egui::pos2(ecran.left(), y), couleur);
        maillage.colored_vertex(egui::pos2(ecran.right(), y), couleur);
        if i > 0 {
            let b = 2 * i as u32;
            maillage.add_triangle(b - 2, b - 1, b);
            maillage.add_triangle(b - 1, b + 1, b);
        }
    }
    peintre.add(maillage);

    // Les lignes de balayage, qui se courbent vers les bords comme sur
    // un tube bombé.
    let trait_ = Stroke::new(1.0_f32, Color32::from_black_alpha(OPACITE_TUBE as u8));
    let mut y = ecran.top().floor() + 0.5;
    while y < ecran.bottom() {
        let points: Vec<Pos2> = (0..=16)
            .map(|i| {
                let x = ecran.left() + ecran.width() * i as f32 / 16.0;
                egui::pos2(x, y + courbure(ecran, egui::pos2(x, y)))
            })
            .collect();
        peintre.add(egui::Shape::line(points, trait_));
        y += ECART_TUBE;
    }

    // Les bords assombris : une grille de points dont l'ombre grandit
    // vers les bords (egui adoucit entre les points).
    const N: usize = 12;
    let mut ombre = epaint::Mesh::default();
    for j in 0..=N {
        for i in 0..=N {
            let p = egui::pos2(
                ecran.left() + ecran.width() * i as f32 / N as f32,
                ecran.top() + ecran.height() * j as f32 / N as f32,
            );
            let alpha = (OPACITE_VIGNETTE * vignette(ecran, p)).round() as u8;
            ombre.colored_vertex(p, Color32::from_black_alpha(alpha));
        }
    }
    let ligne = (N + 1) as u32;
    for j in 0..N as u32 {
        for i in 0..N as u32 {
            let a = j * ligne + i;
            ombre.add_triangle(a, a + 1, a + ligne);
            ombre.add_triangle(a + 1, a + ligne + 1, a + ligne);
        }
    }
    peintre.add(ombre);

    // Les coins arrondis noirs du tube.
    for (coin, centre, angle) in [
        (
            ecran.left_top(),
            ecran.left_top() + egui::vec2(RAYON_COINS, RAYON_COINS),
            180.0_f32,
        ),
        (
            ecran.right_top(),
            ecran.right_top() + egui::vec2(-RAYON_COINS, RAYON_COINS),
            270.0,
        ),
        (
            ecran.right_bottom(),
            ecran.right_bottom() - egui::vec2(RAYON_COINS, RAYON_COINS),
            0.0,
        ),
        (
            ecran.left_bottom(),
            ecran.left_bottom() + egui::vec2(RAYON_COINS, -RAYON_COINS),
            90.0,
        ),
    ] {
        let mut noir = epaint::Mesh::default();
        noir.colored_vertex(coin, Color32::BLACK);
        for k in 0..=8 {
            let a = (angle + 90.0 * k as f32 / 8.0).to_radians();
            noir.colored_vertex(
                centre + RAYON_COINS * egui::vec2(a.cos(), a.sin()),
                Color32::BLACK,
            );
            if k > 0 {
                noir.add_triangle(0, k, k + 1);
            }
        }
        peintre.add(noir);
    }
}

/// De combien de points la ligne de balayage se décale en `p` : rien au
/// milieu, jusqu'à `BOMBE` aux coins, toujours vers le milieu de l'écran.
fn courbure(ecran: Rect, p: Pos2) -> f32 {
    let x = (p.x - ecran.center().x) / (ecran.width() / 2.0);
    let y = (p.y - ecran.center().y) / (ecran.height() / 2.0);
    -BOMBE * y * x * x
}

/// L'ombre des bords en `p`, de 0 (dans tout le milieu) à 1 (aux coins).
fn vignette(ecran: Rect, p: Pos2) -> f32 {
    let x = (p.x - ecran.center().x) / (ecran.width() / 2.0);
    let y = (p.y - ecran.center().y) / (ecran.height() / 2.0);
    // Distance « arrondie » au centre : 1 au milieu des bords, plus aux coins.
    let d = (x.powi(4) + y.powi(4)).powf(0.25);
    let t = ((d - 0.55) / 0.5).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Le scintillement à `temps`, entre 0 et 1, irrégulier comme un vieux tube.
fn scintillement(temps: f32) -> f32 {
    use std::f32::consts::TAU;
    let s = 0.6 * (temps * TAU * 7.3).sin() + 0.4 * (temps * TAU * 13.1).sin();
    0.5 + 0.5 * s
}

/// Le haut de la bande lumineuse à `temps` : elle part au-dessus de l'écran
/// et descend jusqu'en bas en `PERIODE_BANDE` secondes, puis recommence.
fn position_bande(temps: f32, hauteur: f32) -> f32 {
    (temps % PERIODE_BANDE) / PERIODE_BANDE * (hauteur + HAUTEUR_BANDE) - HAUTEUR_BANDE
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

/// Lit le fichier des lignes d'écran : « non », « cathodique », et sinon
/// les simples lignes (comme avant l'écran cathodique).
fn ecran_depuis(texte: Option<&str>) -> Ecran {
    match texte.map(str::trim) {
        Some("non") => Ecran::Aucun,
        Some("cathodique") => Ecran::Cathodique,
        _ => Ecran::Lignes,
    }
}

/// Ce qu'on écrit dans le fichier des lignes d'écran.
fn texte_ecran(ecran: Ecran) -> &'static str {
    match ecran {
        Ecran::Aucun => "non",
        Ecran::Lignes => "oui",
        Ecran::Cathodique => "cathodique",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_reglages_se_relisent() {
        assert_eq!(ecran_depuis(None), Ecran::Lignes);
        assert_eq!(ecran_depuis(Some("oui\n")), Ecran::Lignes);
        assert_eq!(ecran_depuis(Some("non\n")), Ecran::Aucun);
        assert_eq!(ecran_depuis(Some("cathodique\n")), Ecran::Cathodique);
        for ecran in [Ecran::Aucun, Ecran::Lignes, Ecran::Cathodique] {
            assert_eq!(ecran_depuis(Some(texte_ecran(ecran))), ecran);
        }
    }

    #[test]
    fn le_tube_est_bombe() {
        let ecran = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(200.0, 100.0));
        assert_eq!(courbure(ecran, ecran.center()), 0.0, "droit au milieu");
        assert_eq!(courbure(ecran, egui::pos2(100.0, 0.0)), 0.0);
        // Aux coins, les lignes se rapprochent du milieu.
        assert_eq!(
            courbure(ecran, egui::pos2(0.0, 0.0)),
            BOMBE,
            "en haut : vers le bas"
        );
        assert_eq!(courbure(ecran, egui::pos2(200.0, 100.0)), -BOMBE);
    }

    #[test]
    fn les_bords_sont_assombris() {
        let ecran = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(200.0, 100.0));
        assert_eq!(vignette(ecran, ecran.center()), 0.0, "milieu net");
        assert_eq!(vignette(ecran, egui::pos2(60.0, 40.0)), 0.0);
        assert_eq!(vignette(ecran, egui::pos2(0.0, 0.0)), 1.0, "coins sombres");
        let bord = vignette(ecran, egui::pos2(100.0, 0.0));
        assert!(bord > 0.5 && bord < 1.0, "bords à moitié : {bord}");
    }

    #[test]
    fn le_scintillement_reste_leger() {
        for i in 0..1000 {
            let s = scintillement(i as f32 * 0.013);
            assert!((0.0..=1.0).contains(&s));
        }
        assert!(
            position_bande(0.0, 500.0) <= -HAUTEUR_BANDE + 1e-3,
            "part au-dessus"
        );
        assert!(
            position_bande(PERIODE_BANDE - 0.01, 500.0) > 495.0,
            "finit en bas"
        );
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
            ecran: Ecran::Cathodique,
            phase: 0.0,
            temps: 0.0,
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
        assert_eq!(effets.temps, PERIODE / 2.0, "le tube ne scintille plus");
    }
}
