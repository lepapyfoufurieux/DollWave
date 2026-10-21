//! Les polices d'écriture de l'application, toutes sous licence libre SIL OFL
//! (voir les fichiers *-LICENSE.txt dans assets/polices) :
//!
//! - Orbitron : les titres (en-têtes de section), en majuscules ;
//! - Rajdhani : tout le texte courant ;
//! - Share Tech Mono : les chiffres (temps, durées, compteur, volume) ;
//! - Inter : en secours, pour les caractères absents des trois autres
//!   (flèches ← →, étoiles ★ ☆, alphabets non latins…).
//!
//! Elles sont intégrées dans l'exécutable : rien à installer sur l'ordinateur,
//! sous Windows comme sous Linux.

use eframe::egui;

const ORBITRON: &[u8] = include_bytes!("../assets/polices/Orbitron-Bold.ttf");
const RAJDHANI: &[u8] = include_bytes!("../assets/polices/Rajdhani-Medium.ttf");
const SHARE_TECH_MONO: &[u8] = include_bytes!("../assets/polices/ShareTechMono-Regular.ttf");
const INTER: &[u8] = include_bytes!("../assets/polices/Inter-Regular.ttf");

/// Le nom de la famille des titres (Orbitron).
const TITRES: &str = "Titres";

/// Installe les polices et les tailles de texte, au démarrage.
pub fn installer(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
    ctx.style_mut(|style| {
        use egui::{FontId, TextStyle};
        // Rajdhani est plus étroite et un peu plus petite qu'Inter à taille
        // égale : on agrandit le texte pour garder la même lisibilité.
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(14.5)),
            (TextStyle::Button, FontId::proportional(14.5)),
            (TextStyle::Monospace, FontId::monospace(14.0)),
            (TextStyle::Heading, FontId::new(14.0, famille_titres())),
        ]
        .into();
    });
}

/// La liste des polices de chaque famille, de la préférée au dernier secours.
///
/// Les polices d'émojis d'egui restent en dernier : les icônes 🔀 🔁 💾 🗑…
/// continuent de s'afficher comme avant.
fn definitions() -> egui::FontDefinitions {
    let mut polices = egui::FontDefinitions::default();
    for (nom, octets) in [
        ("Orbitron", ORBITRON),
        ("Rajdhani", RAJDHANI),
        ("Share Tech Mono", SHARE_TECH_MONO),
        ("Inter", INTER),
    ] {
        polices
            .font_data
            .insert(nom.to_owned(), egui::FontData::from_static(octets));
    }

    // Les secours d'egui (Ubuntu, émojis), derrière Inter.
    let mut secours = polices.families[&egui::FontFamily::Proportional].clone();
    secours.insert(0, "Inter".to_owned());

    let famille = |premiere: &str| {
        let mut liste = vec![premiere.to_owned()];
        liste.extend(secours.iter().cloned());
        liste
    };
    polices
        .families
        .insert(egui::FontFamily::Proportional, famille("Rajdhani"));
    polices
        .families
        .insert(famille_titres(), famille("Orbitron"));
    // Pour les chiffres : Share Tech Mono, puis Inter et les secours
    // habituels d'egui (Hack, émojis).
    let chiffres = polices
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default();
    chiffres.insert(0, "Share Tech Mono".to_owned());
    chiffres.insert(1, "Inter".to_owned());
    polices
}

/// La famille des titres (Orbitron), pour les grands titres dessinés à la main.
pub fn famille_titres() -> egui::FontFamily {
    egui::FontFamily::Name(TITRES.into())
}

/// Un en-tête de section (« BIBLIOTHÈQUE ») : Orbitron, en majuscules espacées.
pub fn titre(texte: &str) -> egui::RichText {
    egui::RichText::new(texte.to_uppercase())
        .text_style(egui::TextStyle::Heading)
        .extra_letter_spacing(1.0)
}

/// Des chiffres (temps, durée, compteur, volume) : Share Tech Mono.
pub fn chiffres(texte: impl Into<String>) -> egui::RichText {
    egui::RichText::new(texte).monospace()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un contexte egui avec nos polices, prêt à mesurer du texte.
    fn contexte() -> egui::Context {
        let ctx = egui::Context::default();
        installer(&ctx);
        // Les polices ne sont prêtes qu'une fois une image commencée.
        let _ = ctx.run(Default::default(), |_| {});
        ctx
    }

    #[test]
    fn chaque_famille_commence_par_sa_police_puis_inter() {
        let polices = definitions();
        let familles = [
            (egui::FontFamily::Proportional, "Rajdhani"),
            (famille_titres(), "Orbitron"),
            (egui::FontFamily::Monospace, "Share Tech Mono"),
        ];
        for (famille, premiere) in familles {
            let liste = &polices.families[&famille];
            assert_eq!(liste[0], premiere);
            assert!(
                liste.contains(&"Inter".to_owned()),
                "{famille:?} sans Inter"
            );
            assert!(
                liste.contains(&"NotoEmoji-Regular".to_owned()),
                "{famille:?} sans les émojis"
            );
        }
        assert_eq!(
            polices.families[&egui::FontFamily::Proportional][1],
            "Inter"
        );
        assert_eq!(polices.families[&famille_titres()][1], "Inter");
    }

    #[test]
    fn toutes_les_familles_affichent_accents_fleches_etoiles_et_emojis() {
        let ctx = contexte();
        let familles = [
            egui::FontFamily::Proportional,
            famille_titres(),
            egui::FontFamily::Monospace,
        ];
        ctx.fonts(|f| {
            for famille in familles {
                let police = egui::FontId::new(14.0, famille.clone());
                for texte in [
                    "àâäçéèêëîïôöùûüÿœæ ÀÂÄÇÉÈÊËÎÏÔÖÙÛÜŸŒÆ « » ’ – … · %",
                    "BIBLIOTHÈQUE PLAYLISTS LISTE DE LECTURE Édith Œuvres Björk",
                    "← → ★ ☆ Кино",
                    "🔀 🔁 💾 🗑 ➕ ✖ ✏ ⌨ ⏭ 📂 📁 ✔ ⚙",
                    "Titre 3 / 12 · 47 min 1:07 / 3:45 50 %",
                ] {
                    assert!(f.has_glyphs(&police, texte), "{famille:?} : {texte}");
                }
            }
        });
    }

    #[test]
    fn les_titres_sont_en_majuscules_et_en_orbitron() {
        let ctx = contexte();
        assert_eq!(titre("Bibliothèque").text(), "BIBLIOTHÈQUE");
        let style = ctx.style();
        assert_eq!(
            style.text_styles[&egui::TextStyle::Heading].family,
            famille_titres()
        );
    }
}
