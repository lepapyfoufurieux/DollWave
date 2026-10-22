//! La fenêtre sans le cadre du système : le lecteur dessine lui-même sa barre
//! de titre (voir bandeau.rs), son liseré cyan et ses bords à tirer pour la
//! redimensionner.

use eframe::egui::{self, CursorIcon, Pos2, Rect, ResizeDirection, Stroke, ViewportCommand};

use crate::theme::{self, CYAN};

/// L'épaisseur des bords qu'on tire pour redimensionner, en points.
const EPAISSEUR: f32 = 5.0;
/// La taille des coins, un peu plus grands pour être faciles à attraper.
const COIN: f32 = 12.0;

/// Le bord (ou le coin) de `fenetre` sous `pos`, s'il y en a un.
pub fn bord(pos: Pos2, fenetre: Rect) -> Option<ResizeDirection> {
    if !fenetre.contains(pos) {
        return None;
    }
    let (gauche, droite) = (pos.x - fenetre.left(), fenetre.right() - pos.x);
    let (haut, bas) = (pos.y - fenetre.top(), fenetre.bottom() - pos.y);
    let pres = |d: f32| d < EPAISSEUR;
    let coin = |d: f32| d < COIN;
    use ResizeDirection::*;
    let direction = if (pres(haut) && coin(gauche)) || (pres(gauche) && coin(haut)) {
        NorthWest
    } else if (pres(haut) && coin(droite)) || (pres(droite) && coin(haut)) {
        NorthEast
    } else if (pres(bas) && coin(gauche)) || (pres(gauche) && coin(bas)) {
        SouthWest
    } else if (pres(bas) && coin(droite)) || (pres(droite) && coin(bas)) {
        SouthEast
    } else if pres(haut) {
        North
    } else if pres(bas) {
        South
    } else if pres(gauche) {
        West
    } else if pres(droite) {
        East
    } else {
        return None;
    };
    Some(direction)
}

fn curseur(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}

/// Agrandit la fenêtre, ou la remet comme avant si elle l'est déjà.
pub fn agrandir_ou_restaurer(ctx: &egui::Context) {
    let agrandie = ctx.input(|i| i.viewport().maximized == Some(true));
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!agrandie));
}

/// À appeler à chaque image, avant les panneaux : le liseré cyan autour de la
/// fenêtre, et les bords qui la redimensionnent (sauf quand elle est agrandie).
pub fn bords(ctx: &egui::Context) {
    let fenetre = ctx.screen_rect();
    let agrandie = ctx.input(|i| i.viewport().maximized == Some(true));

    // Le liseré, par-dessus tout le reste.
    if !agrandie {
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("liseré de la fenêtre"),
        ))
        .rect_stroke(
            fenetre.shrink(0.5),
            0.0,
            Stroke::new(1.0_f32, theme::attenue(CYAN, 200)),
        );
        // Les bords sont pris avant les panneaux : le séparateur de la
        // bibliothèque ou une barre de défilement collée au bord ne passent
        // pas devant.
        let pos = ctx.input(|i| i.pointer.hover_pos());
        if let Some(direction) = pos.and_then(|pos| bord(pos, fenetre)) {
            ctx.set_cursor_icon(curseur(direction));
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
            }
        }
    }
}

/// Vrai si le pointeur est sur un bord qui redimensionne : les clics y sont
/// pour la fenêtre, pas pour ce qui est dessous.
pub fn sur_un_bord(ctx: &egui::Context) -> bool {
    let agrandie = ctx.input(|i| i.viewport().maximized == Some(true));
    !agrandie
        && ctx
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|pos| bord(pos, ctx.screen_rect()).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ResizeDirection::*;

    fn fenetre() -> Rect {
        Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 800.0))
    }

    #[test]
    fn les_bords_et_les_coins_sont_reconnus() {
        let f = fenetre();
        assert!(matches!(bord(egui::pos2(500.0, 1.0), f), Some(North)));
        assert!(matches!(bord(egui::pos2(500.0, 798.0), f), Some(South)));
        assert!(matches!(bord(egui::pos2(2.0, 400.0), f), Some(West)));
        assert!(matches!(bord(egui::pos2(997.0, 400.0), f), Some(East)));
        assert!(matches!(bord(egui::pos2(1.0, 1.0), f), Some(NorthWest)));
        // Les coins débordent un peu le long des bords.
        assert!(matches!(bord(egui::pos2(10.0, 2.0), f), Some(NorthWest)));
        assert!(matches!(bord(egui::pos2(998.0, 790.0), f), Some(SouthEast)));
        assert!(matches!(bord(egui::pos2(990.0, 2.0), f), Some(NorthEast)));
        assert!(matches!(bord(egui::pos2(2.0, 795.0), f), Some(SouthWest)));
    }

    #[test]
    fn le_milieu_et_le_dehors_ne_redimensionnent_pas() {
        let f = fenetre();
        assert!(bord(egui::pos2(500.0, 400.0), f).is_none());
        assert!(bord(egui::pos2(500.0, 6.0), f).is_none());
        assert!(bord(egui::pos2(-1.0, 400.0), f).is_none());
        assert!(bord(egui::pos2(500.0, 801.0), f).is_none());
    }
}
