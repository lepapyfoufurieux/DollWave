//! Le thème « Néon » de l'interface : la palette de couleurs et le
//! quadrillage du fond (voir design/interface-neon.md).
//!
//! Toutes les couleurs de l'interface sont ici, pour que les prochaines
//! étapes (polices, cadres, HUD…) piochent au même endroit. Le thème est
//! toujours sombre, même si Windows est réglé en mode clair.

mod formes;

pub use formes::*;

use eframe::egui::{self, Color32, Stroke};

/// Le fond de la fenêtre : bleu nuit presque noir.
pub const FOND: Color32 = Color32::from_rgb(0x07, 0x07, 0x0F);
/// Le fond des panneaux (bibliothèque, menus, fenêtres).
pub const PANNEAU: Color32 = Color32::from_rgb(0x0E, 0x0D, 0x1F);
/// Le néon principal.
pub const CYAN: Color32 = Color32::from_rgb(0x00, 0xE5, 0xFF);
/// Le néon secondaire.
pub const MAGENTA: Color32 = Color32::from_rgb(0xFF, 0x2B, 0xD6);
/// L'accent : favoris ★, et plus tard le titre en cours.
pub const JAUNE: Color32 = Color32::from_rgb(0xF4, 0xFF, 0x3A);
/// Le texte courant.
pub const TEXTE: Color32 = Color32::from_rgb(0xE8, 0xF8, 0xFF);
/// Le texte secondaire (infos, aides).
pub const TEXTE_DISCRET: Color32 = Color32::from_rgb(0x8B, 0x9A, 0xC2);
/// Les messages d'erreur : un rose-rouge qui reste lisible sur le fond nuit.
pub const ERREUR: Color32 = Color32::from_rgb(0xFF, 0x5A, 0x7A);

/// Le fond des boutons et des cases, un cran au-dessus des panneaux.
const BOUTON: Color32 = Color32::from_rgb(0x16, 0x15, 0x33);
/// Un bouton survolé.
const BOUTON_SURVOLE: Color32 = Color32::from_rgb(0x1F, 0x1D, 0x48);
/// Un bouton enfoncé.
const BOUTON_ENFONCE: Color32 = Color32::from_rgb(0x0B, 0x3A, 0x4A);
/// Le fond des zones de saisie et du visualiseur, plus sombre que le fond.
const CREUX: Color32 = Color32::from_rgb(0x04, 0x04, 0x0A);
/// Ce qui est sélectionné (titre en cours, 🔀 / 🔁 allumés, onglet choisi) :
/// un magenta foncé, pour que le texte blanc par-dessus reste lisible.
const SELECTION: Color32 = Color32::from_rgb(0x7A, 0x14, 0x66);

/// L'écart entre deux lignes du quadrillage, en points.
const MAILLE: f32 = 28.0;
/// Une ligne sur 4 est un peu plus marquée, comme sur un écran technique.
const MAILLE_FORTE: usize = 4;

/// Un néon atténué : `opacite` de 0 (invisible) à 255 (plein).
pub fn attenue(couleur: Color32, opacite: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(couleur.r(), couleur.g(), couleur.b(), opacite)
}

/// Les couleurs egui du thème Néon.
pub fn visuels() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.dark_mode = true;
    v.panel_fill = attenue(PANNEAU, 230); // le quadrillage transparaît à peine
    v.window_fill = PANNEAU;
    v.window_stroke = Stroke::new(1.0_f32, attenue(CYAN, 110));
    v.extreme_bg_color = CREUX;
    v.faint_bg_color = attenue(CYAN, 6);
    v.code_bg_color = BOUTON;
    v.hyperlink_color = CYAN;
    v.warn_fg_color = JAUNE;
    v.error_fg_color = ERREUR;
    v.selection.bg_fill = SELECTION;
    v.selection.stroke = Stroke::new(1.0_f32, TEXTE);
    v.text_cursor.stroke = Stroke::new(2.0_f32, CYAN);

    let w = &mut v.widgets;
    // Textes, séparateurs, lignes de l'arbre.
    w.noninteractive.bg_fill = PANNEAU;
    w.noninteractive.weak_bg_fill = PANNEAU;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, attenue(CYAN, 45));
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXTE);
    // Boutons au repos : contour cyan discret.
    w.inactive.bg_fill = BOUTON;
    w.inactive.weak_bg_fill = BOUTON;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, attenue(CYAN, 70));
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXTE);
    // Survolés : le contour s'allume.
    w.hovered.bg_fill = BOUTON_SURVOLE;
    w.hovered.weak_bg_fill = BOUTON_SURVOLE;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, CYAN);
    w.hovered.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
    // Enfoncés.
    w.active.bg_fill = BOUTON_ENFONCE;
    w.active.weak_bg_fill = BOUTON_ENFONCE;
    w.active.bg_stroke = Stroke::new(1.0_f32, CYAN);
    w.active.fg_stroke = Stroke::new(2.0_f32, Color32::WHITE);
    // Menu déroulant ouvert : en magenta.
    w.open.bg_fill = BOUTON;
    w.open.weak_bg_fill = BOUTON_SURVOLE;
    w.open.bg_stroke = Stroke::new(1.0_f32, MAGENTA);
    w.open.fg_stroke = Stroke::new(1.0_f32, TEXTE);
    v
}

/// Installe le thème au démarrage. Les deux variantes d'egui (claire et
/// sombre) reçoivent les couleurs Néon : l'interface reste sombre quel que
/// soit le réglage de Windows.
pub fn installer(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuels());
    ctx.set_visuals_of(egui::Theme::Light, visuels());
}

/// Le cadre de la zone centrale : transparent, pour laisser voir le
/// quadrillage du fond.
pub fn cadre_central(style: &egui::Style) -> egui::Frame {
    egui::Frame::central_panel(style).fill(Color32::TRANSPARENT)
}

/// Le cadre d'un panneau rangé dans un autre (les playlists sous la
/// bibliothèque) : transparent, sinon les deux fonds s'empileraient.
pub fn cadre_interieur(style: &egui::Style) -> egui::Frame {
    egui::Frame::side_top_panel(style).fill(Color32::TRANSPARENT)
}

/// Dessine le fond nuit et son quadrillage sous tous les panneaux. À appeler
/// au début de chaque image, avant les panneaux.
///
/// Une centaine de traits pour une grande fenêtre : rien de mesurable.
pub fn dessiner_fond(ctx: &egui::Context) {
    let ecran = ctx.screen_rect();
    let peintre = ctx.layer_painter(egui::LayerId::background());
    peintre.rect_filled(ecran, 0.0, FOND);
    let faible = Stroke::new(1.0_f32, attenue(CYAN, 6));
    let forte = Stroke::new(1.0_f32, attenue(CYAN, 14));
    for (n, x) in positions(ecran.left(), ecran.right()).enumerate() {
        let trait_ = if n % MAILLE_FORTE == 0 { forte } else { faible };
        peintre.vline(x, ecran.y_range(), trait_);
    }
    for (n, y) in positions(ecran.top(), ecran.bottom()).enumerate() {
        let trait_ = if n % MAILLE_FORTE == 0 { forte } else { faible };
        peintre.hline(ecran.x_range(), y, trait_);
    }
}

/// Les positions des lignes du quadrillage entre `debut` et `fin`, une toutes
/// les `MAILLE` points. Elles sont posées au milieu d'un pixel pour rester nettes.
fn positions(debut: f32, fin: f32) -> impl Iterator<Item = f32> {
    let nombre = ((fin - debut) / MAILLE).max(0.0) as usize + 1;
    (0..nombre).map(move |n| debut + n as f32 * MAILLE + 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Luminance relative (WCAG) d'une couleur opaque.
    fn luminance(c: Color32) -> f32 {
        let canal = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * canal(c.r()) + 0.7152 * canal(c.g()) + 0.0722 * canal(c.b())
    }

    fn contraste(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn le_theme_est_sombre_meme_si_windows_est_en_clair() {
        let ctx = egui::Context::default();
        installer(&ctx);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let v = ctx.style_of(theme).visuals.clone();
            assert!(v.dark_mode);
            assert_eq!(v.window_fill, PANNEAU);
        }
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::Dark
        );
    }

    #[test]
    fn les_textes_restent_lisibles() {
        // 4,5 : le minimum recommandé pour du texte courant.
        assert!(contraste(TEXTE, FOND) > 4.5);
        assert!(contraste(TEXTE, PANNEAU) > 4.5);
        assert!(contraste(TEXTE, BOUTON) > 4.5);
        assert!(contraste(TEXTE_DISCRET, PANNEAU) > 4.5);
        assert!(contraste(TEXTE, SELECTION) > 4.5);
        assert!(contraste(Color32::WHITE, BOUTON_SURVOLE) > 4.5);
        assert!(contraste(Color32::WHITE, BOUTON_ENFONCE) > 4.5);
        assert!(contraste(JAUNE, PANNEAU) > 4.5);
        assert!(contraste(ERREUR, PANNEAU) > 4.5);
    }

    #[test]
    fn le_quadrillage_couvre_l_ecran_sans_trop_de_traits() {
        let xs: Vec<f32> = positions(0.0, 1400.0).collect();
        assert_eq!(xs.first(), Some(&0.5));
        assert!(*xs.last().unwrap() <= 1400.5);
        assert!(*xs.last().unwrap() > 1400.0 - MAILLE);
        // Grande fenêtre : quelques dizaines de traits seulement.
        assert!(xs.len() + positions(0.0, 840.0).count() < 100);
        assert_eq!(positions(10.0, 5.0).count(), 1);
    }
}
