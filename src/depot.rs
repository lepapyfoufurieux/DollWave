//! Le glisser-déposer : des MP3 ou des dossiers lâchés dans la fenêtre
//! depuis l'Explorateur Windows.

use std::path::PathBuf;

use eframe::egui;

/// Les fichiers et dossiers que l'utilisateur vient de lâcher dans la
/// fenêtre (vide la plupart du temps).
pub fn elements_deposes(ctx: &egui::Context) -> Vec<PathBuf> {
    ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .filter_map(|f| f.path.clone())
            .collect()
    })
}

/// Pendant qu'on survole la fenêtre avec des fichiers, on l'assombrit et on
/// affiche un texte, pour montrer qu'on peut les lâcher.
pub fn dessiner_survol(ctx: &egui::Context) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let calque = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("survol_depot"));
    let peintre = ctx.layer_painter(calque);
    let ecran = ctx.screen_rect();
    peintre.rect_filled(ecran, 0.0, egui::Color32::from_black_alpha(180));
    peintre.text(
        ecran.center(),
        egui::Align2::CENTER_CENTER,
        "Déposez ici pour ajouter à la liste",
        egui::FontId::proportional(22.0),
        egui::Color32::WHITE,
    );
}
