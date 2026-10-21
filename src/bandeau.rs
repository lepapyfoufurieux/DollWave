//! Le bandeau néon en haut de la fenêtre (étape 9), sous la barre de titre
//! de Windows : logo hexagonal, « LECTEUR//MP3 », la version, et les voyants
//! « SYS.OK ▮▮▮▮▯ » qui disent ce qui marche.
//!
//! La vraie barre de titre reste celle du système : la dessiner nous-mêmes
//! (avec Réduire, Agrandir, ✖) est plus risqué et viendra en option plus tard.

use eframe::egui::{self, Color32, Rect, Stroke};

use crate::theme::{self, CYAN, ERREUR, FOND, JAUNE, MAGENTA, TEXTE_DISCRET};

/// La hauteur du bandeau.
pub const HAUTEUR: f32 = 38.0;

/// Ce que disent les voyants, de gauche à droite.
#[derive(Clone, Copy, Default)]
pub struct Voyants {
    /// La sortie son (haut-parleurs, casque…) est ouverte.
    pub sortie: bool,
    /// La bibliothèque est lue et contient de la musique.
    pub bibliotheque: bool,
    /// La liste de lecture n'est pas vide.
    pub liste: bool,
    /// Un morceau est chargé.
    pub morceau: bool,
    /// La musique joue.
    pub lecture: bool,
}

impl Voyants {
    fn tous(&self) -> [(bool, &'static str); 5] {
        [
            (self.sortie, "Sortie son"),
            (self.bibliotheque, "Bibliothèque"),
            (self.liste, "Liste de lecture"),
            (self.morceau, "Morceau chargé"),
            (self.lecture, "Lecture"),
        ]
    }

    /// « SYS.OK ▮▮▮▮▯ » : un ▮ par voyant allumé, un ▯ par voyant éteint.
    /// « SYS.ERR » si aucune sortie son n'a pu être ouverte.
    pub fn texte(&self) -> String {
        let barres: String = self
            .tous()
            .iter()
            .map(|&(allume, _)| if allume { '▮' } else { '▯' })
            .collect();
        let etat = if self.sortie { "SYS.OK" } else { "SYS.ERR" };
        format!("{etat} {barres}")
    }

    /// L'explication des voyants, au survol.
    pub fn explication(&self) -> String {
        self.tous()
            .iter()
            .map(|&(allume, nom)| format!("{} {nom}", if allume { "▮" } else { "▯" }))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Dessine le bandeau dans tout `ui` (un panneau du haut de `HAUTEUR`).
pub fn afficher(ui: &mut egui::Ui, voyants: &Voyants) {
    let zone = ui.max_rect();
    let peintre = ui.painter();

    // Le fond : cyan à gauche, nuit au milieu, magenta à droite.
    let rien = Color32::TRANSPARENT;
    peintre.rect_filled(zone, 0.0, FOND);
    let gauche = Rect::from_min_max(
        zone.min,
        egui::pos2(zone.left() + zone.width() * 0.45, zone.bottom()),
    );
    let droite = Rect::from_min_max(
        egui::pos2(zone.left() + zone.width() * 0.45, zone.top()),
        zone.max,
    );
    theme::degrade(
        peintre,
        &theme::contour(gauche, theme::Coins::AUCUN),
        theme::attenue(CYAN, 38),
        rien,
        egui::vec2(1.0, 0.0),
    );
    theme::degrade(
        peintre,
        &theme::contour(droite, theme::Coins::AUCUN),
        rien,
        theme::attenue(MAGENTA, 38),
        egui::vec2(1.0, 0.0),
    );
    peintre.hline(
        zone.x_range(),
        zone.bottom() - 0.5,
        Stroke::new(1.0_f32, theme::attenue(CYAN, 136)),
    );

    // Le logo : un petit hexagone en dégradé cyan → magenta.
    let milieu = zone.center().y;
    let logo = Rect::from_center_size(
        egui::pos2(zone.left() + 23.0, milieu),
        egui::vec2(18.0, 18.0),
    );
    theme::degrade(
        peintre,
        &theme::hexagone(logo),
        CYAN,
        MAGENTA,
        egui::vec2(1.0, 1.0),
    );

    // « LECTEUR//MP3 » : cyan, les deux barres en magenta.
    let titre = crate::police::titres(ui.ctx(), 14.0);
    let mut x = logo.right() + 14.0;
    for (morceau, couleur) in [("LECTEUR", CYAN), ("//", MAGENTA), ("MP3", CYAN)] {
        let place = theme::texte_lumineux(
            peintre,
            egui::pos2(x, milieu),
            egui::Align2::LEFT_CENTER,
            morceau,
            titre.clone(),
            3.0,
            couleur,
            couleur,
        );
        x = place.right();
    }

    // La version.
    let mono = egui::FontId::monospace(12.0);
    let version = format!("v{} · NEURAL AUDIO DECK", env!("CARGO_PKG_VERSION"));
    let place = peintre.text(
        egui::pos2(x + 14.0, milieu),
        egui::Align2::LEFT_CENTER,
        version,
        mono.clone(),
        TEXTE_DISCRET,
    );

    // Les voyants, tout à droite ; un clic n'y fait rien, le survol explique.
    let couleur = if voyants.sortie { JAUNE } else { ERREUR };
    let etat = peintre.text(
        egui::pos2(zone.right() - 14.0, milieu),
        egui::Align2::RIGHT_CENTER,
        voyants.texte(),
        mono,
        couleur,
    );
    ui.interact(etat, ui.id().with("voyants"), egui::Sense::hover())
        .on_hover_text(voyants.explication());

    // Entre les deux, un trait pointillé cyan.
    let (debut, fin) = (place.right() + 14.0, etat.left() - 14.0);
    let pointille = Stroke::new(1.0_f32, theme::attenue(CYAN, 102));
    let mut x = debut;
    while x + 6.0 <= fin {
        peintre.hline(x..=x + 6.0, milieu, pointille);
        x += 12.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_voyants_disent_ce_qui_marche() {
        let mut voyants = Voyants {
            sortie: true,
            bibliotheque: true,
            liste: true,
            morceau: true,
            lecture: false,
        };
        assert_eq!(voyants.texte(), "SYS.OK ▮▮▮▮▯");
        assert!(voyants.explication().ends_with("▯ Lecture"));
        voyants.sortie = false;
        assert_eq!(voyants.texte(), "SYS.ERR ▯▮▮▮▯");
        assert_eq!(Voyants::default().texte(), "SYS.ERR ▯▯▯▯▯");
    }

    #[test]
    fn le_bandeau_se_dessine_sans_police_installee() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::TopBottomPanel::top("bandeau")
                .exact_height(HAUTEUR)
                .show(ctx, |ui| afficher(ui, &Voyants::default()));
        });
    }
}
