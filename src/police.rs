//! La police d'écriture de l'application : Inter (licence libre SIL OFL,
//! voir assets/polices/Inter-LICENSE.txt).
//!
//! Elle est intégrée dans l'exécutable : rien à installer sur l'ordinateur.
//! Contrairement à la police par défaut d'egui, elle sait afficher les
//! flèches ← → et les étoiles ★ ☆.

use eframe::egui;

const INTER: &[u8] = include_bytes!("../assets/polices/Inter-Regular.ttf");

/// Remplace la police du texte par Inter, au démarrage.
///
/// Les polices d'émojis d'egui restent derrière, en secours : les icônes
/// 🔀 🔁 💾 🗑… continuent de s'afficher comme avant.
pub fn installer(ctx: &egui::Context) {
    let mut polices = egui::FontDefinitions::default();
    polices
        .font_data
        .insert("Inter".to_owned(), egui::FontData::from_static(INTER));
    polices
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".to_owned());
    ctx.set_fonts(polices);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affiche_fleches_etoiles_et_accents() {
        let ctx = egui::Context::default();
        installer(&ctx);
        // Les polices ne sont prêtes qu'une fois une image commencée.
        let _ = ctx.run(Default::default(), |_| {});
        let texte = egui::FontId::proportional(14.0);
        ctx.fonts(|f| {
            assert!(f.has_glyphs(&texte, "← → ★ ☆ Édith Œuvres Björk Кино"));
            assert!(f.has_glyphs(&texte, "🔀 🔁 💾 🗑 ➕ ✖ ✏ ⌨"));
        });
    }
}
