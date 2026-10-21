//! La barre de progression néon, dessinée comme la forme d'onde du morceau
//! (partie jouée en cyan lumineux, reste atténué, curseur jaune), et le
//! volume en 20 segments (cyan → jaune → magenta).

use eframe::egui::{self, Color32, Rect, Stroke};

use crate::theme::{self, CYAN, FOND, JAUNE, MAGENTA};

/// La hauteur de la barre de progression.
pub const HAUTEUR: f32 = 56.0;
/// La largeur d'une barre de la forme d'onde, et l'écart entre deux barres.
const LARGEUR_BARRE: f32 = 3.0;
const ECART_BARRE: f32 = 2.0;
/// La hauteur d'une barre tant qu'on ne connaît pas encore la forme d'onde.
const HAUTEUR_EN_ATTENTE: f32 = 0.12;

/// Le nombre de segments du volume.
pub const SEGMENTS: usize = 20;

/// Combien de barres tiennent dans une largeur donnée.
pub fn nombre_de_barres(largeur: f32) -> usize {
    ((largeur + ECART_BARRE) / (LARGEUR_BARRE + ECART_BARRE))
        .floor()
        .max(1.0) as usize
}

/// Résume la forme d'onde (le plus bas et le plus haut de chaque tranche)
/// en `nombre` barres, de 0 à 1 : la plus forte touche le haut.
pub fn barres_de_l_apercu(apercu: &[(f32, f32)], nombre: usize) -> Vec<f32> {
    if apercu.is_empty() || nombre == 0 {
        return vec![HAUTEUR_EN_ATTENTE; nombre];
    }
    let mut barres: Vec<f32> = (0..nombre)
        .map(|b| {
            let debut = b * apercu.len() / nombre;
            let fin = ((b + 1) * apercu.len() / nombre)
                .max(debut + 1)
                .min(apercu.len());
            apercu[debut..fin]
                .iter()
                .map(|&(bas, haut)| bas.abs().max(haut.abs()))
                .fold(0.0, f32::max)
        })
        .collect();
    let max = barres.iter().copied().fold(0.0, f32::max);
    if max > 0.0 {
        for barre in &mut barres {
            // Jamais tout à fait plate : on voit toujours où est la barre.
            *barre = (*barre / max).max(0.06);
        }
    }
    barres
}

/// La position (de 0 à 1) qui correspond à l'abscisse `x` dans `zone`.
pub fn fraction_sous(zone: Rect, x: f32) -> f32 {
    ((x - zone.left()) / zone.width().max(1.0)).clamp(0.0, 1.0)
}

/// Dessine la forme d'onde dans `zone` : `barres` (rien tant qu'elle n'est
/// pas calculée), `fraction` du morceau déjà jouée, `actif` faux sans morceau.
pub fn dessiner_progression(
    peintre: &egui::Painter,
    zone: Rect,
    barres: Option<&[f32]>,
    fraction: f32,
    actif: bool,
) {
    peintre.rect_filled(zone, 0.0, FOND);
    let bord = Stroke::new(1.0_f32, theme::attenue(CYAN, 50));
    peintre.hline(zone.x_range(), zone.top(), bord);
    peintre.hline(zone.x_range(), zone.bottom(), bord);

    let interieur = zone.shrink2(egui::vec2(4.0, 6.0));
    let nombre = nombre_de_barres(interieur.width());
    let en_attente;
    let barres = match barres {
        Some(b) if b.len() == nombre => b,
        _ => {
            en_attente = vec![HAUTEUR_EN_ATTENTE; nombre];
            &en_attente
        }
    };
    let pas = interieur.width() / nombre as f32;
    let largeur = (pas - ECART_BARRE).max(1.0);
    let jouee = theme::fondu(CYAN, if actif { 1.0 } else { 0.4 });
    let reste = theme::attenue(CYAN, 64);
    let lueur = theme::intensite() > 0.0;
    for (n, &hauteur) in barres.iter().enumerate() {
        let x = interieur.left() + n as f32 * pas;
        let h = (hauteur * interieur.height()).max(2.0);
        let barre = Rect::from_center_size(
            egui::pos2(x + largeur / 2.0, interieur.center().y),
            egui::vec2(largeur, h),
        );
        let deja = (n as f32 + 0.5) / nombre as f32 <= fraction;
        if deja && actif && lueur {
            peintre.rect_filled(barre.expand(1.0), 0.0, theme::attenue(CYAN, 50));
        }
        peintre.rect_filled(barre, 0.0, if deja { jouee } else { reste });
    }

    // Le curseur jaune, avec son petit triangle au-dessus.
    if actif {
        let x = zone.left() + fraction * zone.width();
        let trait_ = Rect::from_min_max(
            egui::pos2(x - 1.0, zone.top() - 6.0),
            egui::pos2(x + 1.0, zone.bottom() + 6.0),
        );
        theme::halo(peintre, trait_, JAUNE, 0.6);
        peintre.rect_filled(trait_, 0.0, JAUNE);
        peintre.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x - 6.0, zone.top() - 10.0),
                egui::pos2(x + 6.0, zone.top() - 10.0),
                egui::pos2(x, zone.top() - 2.0),
            ],
            JAUNE,
            Stroke::NONE,
        ));
    }
}

/// La couleur du segment de volume numéro `n` (cyan, puis jaune, puis magenta).
pub fn couleur_du_segment(n: usize) -> Color32 {
    match n {
        0..=11 => CYAN,
        12..=15 => JAUNE,
        _ => MAGENTA,
    }
}

/// Combien de segments sont allumés pour ce volume (0 à 1).
pub fn segments_allumes(volume: f32) -> usize {
    ((volume.clamp(0.0, 1.0) * SEGMENTS as f32).round() as usize).min(SEGMENTS)
}

/// Le réglage du volume en segments : un clic ou un glissement le change.
/// Renvoie le nouveau volume (0 à 1), arrondi au segment, si on y a touché.
pub fn segments_du_volume(ui: &mut egui::Ui, volume: f32, muet: bool) -> Option<f32> {
    let (largeur, ecart, hauteur) = (6.0, 3.0, 26.0);
    let taille = egui::vec2(SEGMENTS as f32 * (largeur + ecart) - ecart, hauteur);
    let (zone, reponse) = ui.allocate_exact_size(taille, egui::Sense::click_and_drag());
    let reponse = reponse.on_hover_text("Volume : cliquer ou glisser (ou molette)");
    let allumes = if muet { 0 } else { segments_allumes(volume) };
    let peintre = ui.painter();
    for n in 0..SEGMENTS {
        let h = 8.0 + n as f32 * (hauteur - 8.0) / (SEGMENTS - 1) as f32;
        let x = zone.left() + n as f32 * (largeur + ecart);
        let segment = Rect::from_min_max(
            egui::pos2(x, zone.bottom() - h),
            egui::pos2(x + largeur, zone.bottom()),
        );
        let couleur = couleur_du_segment(n);
        if n < allumes {
            if theme::intensite() > 0.0 {
                peintre.rect_filled(segment.expand(1.0), 0.0, theme::attenue(couleur, 60));
            }
            peintre.rect_filled(segment, 0.0, couleur);
        } else {
            peintre.rect_filled(segment, 0.0, theme::attenue(couleur, 48));
        }
    }
    if reponse.clicked() || reponse.dragged() {
        let x = reponse.interact_pointer_pos()?.x;
        // Tout à gauche, on coupe : le premier demi-segment vaut 0.
        let segments = (fraction_sous(zone, x) * SEGMENTS as f32).round();
        return Some(segments / SEGMENTS as f32);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_forme_d_onde_se_resume_en_barres() {
        // 4 tranches : silence, moitié, fort, moitié (en négatif).
        let apercu = [(0.0, 0.0), (-0.2, 0.4), (-0.8, 0.6), (-0.4, 0.1)];
        let barres = barres_de_l_apercu(&apercu, 4);
        assert_eq!(barres.len(), 4);
        assert_eq!(barres[2], 1.0, "la plus forte touche le haut");
        assert!((barres[1] - 0.5).abs() < 1e-6);
        assert!((barres[3] - 0.5).abs() < 1e-6);
        assert!(barres[0] > 0.0, "le silence garde une petite barre");
        // Plus de barres que de tranches : chacune reprend une tranche.
        assert_eq!(barres_de_l_apercu(&apercu, 8).len(), 8);
        // Pas encore calculée : des barres basses, toutes pareilles.
        let attente = barres_de_l_apercu(&[], 5);
        assert!(attente.iter().all(|&b| b == HAUTEUR_EN_ATTENTE));
    }

    #[test]
    fn le_nombre_de_barres_suit_la_largeur() {
        assert_eq!(nombre_de_barres(498.0), 100);
        assert_eq!(nombre_de_barres(0.0), 1);
    }

    #[test]
    fn un_clic_donne_la_position_sous_la_souris() {
        let zone = Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(200.0, 50.0));
        assert_eq!(fraction_sous(zone, 100.0), 0.0);
        assert_eq!(fraction_sous(zone, 200.0), 0.5);
        assert_eq!(fraction_sous(zone, 999.0), 1.0);
        assert_eq!(fraction_sous(zone, 0.0), 0.0);
    }

    #[test]
    fn le_volume_allume_ses_segments_de_cyan_a_magenta() {
        assert_eq!(segments_allumes(0.0), 0);
        assert_eq!(segments_allumes(0.5), 10);
        assert_eq!(segments_allumes(1.0), 20);
        assert_eq!(segments_allumes(3.0), 20);
        assert_eq!(couleur_du_segment(0), CYAN);
        assert_eq!(couleur_du_segment(12), JAUNE);
        assert_eq!(couleur_du_segment(19), MAGENTA);
    }
}
