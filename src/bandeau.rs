//! Le bandeau néon en haut de la fenêtre (étape 9), qui sert aussi de barre
//! de titre : l'icône du lecteur, « DOLL//WAVE », la version, les voyants
//! « SYS.OK ▮▮▮▮▯ » qui disent ce qui marche, puis les boutons Réduire,
//! Agrandir et ✕. On le glisse pour déplacer la fenêtre ; un double-clic
//! l'agrandit.

use eframe::egui::{self, Color32, Rect, Stroke, ViewportCommand};

use crate::fenetre;
use crate::theme::{self, CYAN, ERREUR, FOND, JAUNE, MAGENTA, TEXTE, TEXTE_DISCRET};

/// La hauteur du bandeau.
pub const HAUTEUR: f32 = 38.0;
/// La largeur de chacun des trois boutons de la fenêtre.
const LARGEUR_BOUTON: f32 = 46.0;

/// La taille du logo, en haut à gauche.
const TAILLE_LOGO: f32 = 26.0;

/// L'icône du lecteur, chargée une seule fois puis gardée par egui.
fn texture_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("logo du bandeau");
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let (pixels, largeur, hauteur) = crate::systeme::icone_petite()?;
    let image =
        egui::ColorImage::from_rgba_unmultiplied([largeur as usize, hauteur as usize], &pixels);
    let texture = ctx.load_texture("logo", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

/// Les trois boutons de la fenêtre, à droite du bandeau.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Bouton {
    Reduire,
    Agrandir,
    Fermer,
}

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
    let ctx = ui.ctx().clone();

    // Tout le bandeau sert de barre de titre ; les boutons et les voyants,
    // posés après, passent devant. Sur un bord, c'est le redimensionnement
    // qui gagne (voir fenetre::bords).
    let barre = ui.interact(
        zone,
        ui.id().with("barre de titre"),
        egui::Sense::click_and_drag(),
    );
    if !fenetre::sur_un_bord(&ctx) {
        if barre.double_clicked() {
            fenetre::agrandir_ou_restaurer(&ctx);
        } else if barre.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

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
        theme::attenue(CYAN, 95),
        rien,
        egui::vec2(1.0, 0.0),
    );
    theme::degrade(
        peintre,
        &theme::contour(droite, theme::Coins::AUCUN),
        rien,
        theme::attenue(MAGENTA, 95),
        egui::vec2(1.0, 0.0),
    );
    peintre.hline(
        zone.x_range(),
        zone.bottom() - 0.5,
        Stroke::new(1.0_f32, theme::attenue(CYAN, 200)),
    );

    // Le logo : l'icône du lecteur, la même que dans la barre des tâches.
    let milieu = zone.center().y;
    let logo = Rect::from_center_size(
        egui::pos2(zone.left() + 23.0, milieu),
        egui::vec2(TAILLE_LOGO, TAILLE_LOGO),
    );
    if let Some(texture) = texture_logo(&ctx) {
        peintre.image(
            texture.id(),
            logo,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    // « DOLL//WAVE » : cyan, les deux barres en magenta.
    let titre = crate::police::titres(ui.ctx(), 14.0);
    let mut x = logo.right() + 14.0;
    for (morceau, couleur) in [("DOLL", CYAN), ("//", MAGENTA), ("WAVE", CYAN)] {
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

    // Les boutons de la fenêtre, tout à droite.
    let agrandie = ctx.input(|i| i.viewport().maximized == Some(true));
    let mut droite = zone.right();
    for bouton in [Bouton::Fermer, Bouton::Agrandir, Bouton::Reduire] {
        let place = Rect::from_min_max(
            egui::pos2(droite - LARGEUR_BOUTON, zone.top()),
            egui::pos2(droite, zone.bottom() - 1.0),
        );
        droite = place.left();
        if bouton_de_fenetre(ui, place, bouton, agrandie) && !fenetre::sur_un_bord(&ctx) {
            match bouton {
                Bouton::Reduire => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                Bouton::Agrandir => fenetre::agrandir_ou_restaurer(&ctx),
                // Comme le ✕ de Windows : la fenêtre se range près de
                // l'horloge (voir systeme::ZoneDeNotification::surveiller).
                Bouton::Fermer => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }
    let peintre = ui.painter();

    // Les voyants, à gauche des boutons ; un clic n'y fait rien, le survol
    // explique.
    let couleur = if voyants.sortie { JAUNE } else { ERREUR };
    let etat = peintre.text(
        egui::pos2(droite - 14.0, milieu),
        egui::Align2::RIGHT_CENTER,
        voyants.texte(),
        mono,
        couleur,
    );
    ui.interact(etat, ui.id().with("voyants"), egui::Sense::hover())
        .on_hover_text(voyants.explication());

    // Entre les deux, un encre pointillé cyan.
    let (debut, fin) = (place.right() + 14.0, etat.left() - 14.0);
    let pointille = Stroke::new(1.0_f32, theme::attenue(CYAN, 102));
    let mut x = debut;
    while x + 6.0 <= fin {
        peintre.hline(x..=x + 6.0, milieu, pointille);
        x += 12.0;
    }
}

/// Dessine un bouton de la fenêtre ; vrai s'il vient d'être cliqué.
fn bouton_de_fenetre(ui: &mut egui::Ui, place: Rect, bouton: Bouton, agrandie: bool) -> bool {
    let reponse = ui.interact(
        place,
        ui.id().with(("bouton", bouton as u8)),
        egui::Sense::click(),
    );
    let survol = reponse.hovered();
    let peintre = ui.painter();

    // Le ✕ est toujours sur fond magenta, plus vif au survol ; les deux
    // autres s'allument en cyan au survol.
    let (fond, encre) = match bouton {
        Bouton::Fermer if survol => (MAGENTA, TEXTE),
        Bouton::Fermer => (theme::attenue(MAGENTA, 210), FOND),
        _ if survol => (theme::attenue(CYAN, 60), CYAN),
        _ => (Color32::TRANSPARENT, TEXTE),
    };
    peintre.rect_filled(place, 0.0, fond);
    let encre = Stroke::new(1.2_f32, encre);
    let c = place.center();
    match bouton {
        Bouton::Reduire => {
            peintre.hline(c.x - 5.0..=c.x + 5.0, c.y, encre);
        }
        Bouton::Agrandir if agrandie => {
            // Deux carrés l'un derrière l'autre : « remettre comme avant ».
            let devant = Rect::from_center_size(c + egui::vec2(-1.5, 1.5), egui::vec2(8.0, 8.0));
            peintre.rect_stroke(devant, 0.0, encre);
            peintre.line_segment(
                [
                    devant.left_top() + egui::vec2(3.0, -3.0),
                    devant.right_top() + egui::vec2(3.0, -3.0),
                ],
                encre,
            );
            peintre.line_segment(
                [
                    devant.right_top() + egui::vec2(3.0, -3.0),
                    devant.right_bottom() + egui::vec2(3.0, -3.0),
                ],
                encre,
            );
        }
        Bouton::Agrandir => {
            peintre.rect_stroke(
                Rect::from_center_size(c, egui::vec2(10.0, 10.0)),
                0.0,
                encre,
            );
        }
        Bouton::Fermer => {
            let d = 5.0;
            peintre.line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], encre);
            peintre.line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], encre);
        }
    }

    let infobulle = match bouton {
        Bouton::Reduire => "Réduire",
        Bouton::Agrandir if agrandie => "Restaurer",
        Bouton::Agrandir => "Agrandir",
        Bouton::Fermer => "Ranger près de l'horloge",
    };
    reponse.on_hover_text(infobulle).clicked()
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
