//! La fenêtre sans le cadre du système : le lecteur dessine lui-même sa barre
//! de titre (voir bandeau.rs), son liseré cyan et ses bords à tirer pour la
//! redimensionner.

use eframe::egui::{
    self, panel::PanelState, CursorIcon, Pos2, Rect, ResizeDirection, Stroke, ViewportCommand,
};

use crate::mini::TAILLE_MIN_GRANDE;
use crate::theme::{self, CYAN};

/// L'épaisseur des bords qu'on tire pour redimensionner, en points.
const EPAISSEUR: f32 = 5.0;
/// La taille des coins, un peu plus grands pour être faciles à attraper.
const COIN: f32 = 12.0;

/// Les panneaux dont on tire le séparateur : la bibliothèque (sa largeur) et
/// les playlists sous elle (leur hauteur).
pub const PANNEAUX: [&str; 2] = ["bibliotheque", "playlists"];

/// Les tailles des panneaux à séparateur, notées avant de les dessiner.
///
/// Réduite, la fenêtre continue de se dessiner (la musique avance), mais
/// Windows lui donne une toute petite taille. egui rétrécit alors les
/// panneaux pour qu'ils y tiennent, et retient ces tailles : à la
/// réouverture, les séparateurs avaient bougé. On remet donc celles d'avant.
pub struct Separateurs(Vec<(egui::Id, Option<PanelState>)>);

impl Separateurs {
    /// À appeler avant de dessiner les panneaux.
    pub fn noter(ctx: &egui::Context) -> Self {
        Self(
            PANNEAUX
                .iter()
                .map(|&nom| {
                    let id = egui::Id::new(nom);
                    (id, PanelState::load(ctx, id))
                })
                .collect(),
        )
    }

    /// À appeler après : si la fenêtre est réduite, les tailles notées
    /// reviennent.
    pub fn garder_si_reduite(self, ctx: &egui::Context) {
        if !est_reduite(ctx) {
            return;
        }
        ctx.data_mut(|d| {
            for (id, etat) in self.0 {
                match etat {
                    Some(etat) => d.insert_persisted(id, etat),
                    None => {
                        d.remove::<PanelState>(id);
                    }
                }
            }
        });
    }
}

/// Vrai si la fenêtre est réduite, ou bien plus petite que la taille minimale
/// de la grande fenêtre (réduite, ou à l'instant où elle quitte le
/// mini-lecteur ; avec une marge pour les écrans zoomés).
pub fn est_reduite(ctx: &egui::Context) -> bool {
    let taille = ctx.screen_rect().size();
    ctx.input(|i| i.viewport().minimized == Some(true))
        || taille.x < TAILLE_MIN_GRANDE.x / 2.0
        || taille.y < TAILLE_MIN_GRANDE.y / 2.0
}

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
    use egui::Vec2;
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

    /// Les panneaux comme dans le lecteur (main.rs), avec leurs séparateurs.
    fn dessiner_les_panneaux(ctx: &egui::Context) {
        let separateurs = Separateurs::noter(ctx);
        egui::SidePanel::left(PANNEAUX[0])
            .resizable(true)
            .default_width(310.0)
            .width_range(220.0..=600.0)
            .show(ctx, |ui| {
                egui::TopBottomPanel::bottom(PANNEAUX[1])
                    .resizable(true)
                    .default_height(240.0)
                    .height_range(110.0..=500.0)
                    .show_inside(ui, |ui| ui.set_min_height(ui.available_height()));
            });
        egui::CentralPanel::default().show(ctx, |_| {});
        separateurs.garder_si_reduite(ctx);
    }

    fn image(ctx: &egui::Context, taille: Vec2, reduite: bool) {
        let mut entree = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, taille)),
            ..Default::default()
        };
        entree
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .minimized = Some(reduite);
        let _ = ctx.run(entree, dessiner_les_panneaux);
    }

    fn tailles(ctx: &egui::Context) -> (f32, f32) {
        let taille = |nom| PanelState::load(ctx, egui::Id::new(nom)).unwrap().size();
        (taille(PANNEAUX[0]).x, taille(PANNEAUX[1]).y)
    }

    /// Le bug : réduire puis rouvrir la fenêtre faisait bouger les séparateurs
    /// (Windows donne une toute petite taille à la fenêtre réduite).
    #[test]
    fn reduire_puis_rouvrir_ne_bouge_pas_les_separateurs() {
        let ctx = egui::Context::default();
        let grande = egui::vec2(1320.0, 900.0);
        // Des séparateurs déjà tirés par l'utilisateur.
        ctx.data_mut(|d| {
            for (nom, taille) in [
                (PANNEAUX[0], egui::vec2(450.0, 860.0)),
                (PANNEAUX[1], egui::vec2(450.0, 380.0)),
            ] {
                let rect = Rect::from_min_size(Pos2::ZERO, taille);
                d.insert_persisted(egui::Id::new(nom), PanelState { rect });
            }
        });
        image(&ctx, grande, false);
        let avant = tailles(&ctx);
        assert_eq!(avant.0.round(), 450.0);
        // Réduite : la taille que Windows donne aux fenêtres réduites.
        for _ in 0..3 {
            image(&ctx, egui::vec2(160.0, 28.0), true);
        }
        image(&ctx, grande, false);
        assert_eq!(tailles(&ctx), avant);
    }

    /// Une fenêtre lancée réduite garde les tailles par défaut.
    #[test]
    fn lancee_reduite_les_separateurs_restent_par_defaut() {
        let ctx = egui::Context::default();
        image(&ctx, egui::vec2(160.0, 28.0), true);
        image(&ctx, egui::vec2(1320.0, 900.0), false);
        assert_eq!(tailles(&ctx).0.round(), 310.0);
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
