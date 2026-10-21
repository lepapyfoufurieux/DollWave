//! Le bloc « En lecture » de l'interface néon (le HUD) : l'état de la
//! lecture, le titre en grand, l'artiste, l'album, et à droite les infos
//! techniques (débit, fréquence, volume égalisé, sortie son).

use eframe::egui::{self, Color32, Rect, Stroke};

use crate::infos_fichier::{Debit, Infos};
use crate::theme::{self, Coins, CYAN, JAUNE, MAGENTA, PANNEAU, TEXTE, TEXTE_DISCRET};

/// La hauteur du bloc.
const HAUTEUR: f32 = 118.0;
/// La largeur de la colonne des infos techniques, à droite.
const LARGEUR_INFOS: f32 = 240.0;

/// Ce que montre le bloc.
pub struct EnLecture {
    /// « EN LECTURE », « EN PAUSE »…
    pub etat: String,
    /// Le voyant jaune s'allume pendant la lecture.
    pub voyant: bool,
    /// « TITRE 3 / 12 », s'il y a une liste.
    pub compteur: Option<String>,
    pub titre: String,
    pub artiste: Option<String>,
    pub album: Option<String>,
    /// Les lignes des infos techniques : (libellé, valeur).
    pub techniques: Vec<(&'static str, String)>,
}

/// Les infos techniques du morceau en cours, comme dans la maquette :
/// « DÉBIT 320 KBPS », « FRÉQ. 44,1 KHZ », « VOLUME ÉGAL. −4,8 DB »,
/// « SORTIE CASQUE USB ». `gain` : le facteur du volume égalisé (rien si
/// l'option est coupée, `Some(None)` s'il n'est pas encore mesuré).
pub fn infos_techniques(
    infos: Option<&Infos>,
    gain: Option<Option<f32>>,
    sortie: Option<&str>,
) -> Vec<(&'static str, String)> {
    let qualite = infos.and_then(|i| i.qualite.as_ref());
    let debit = match qualite.map(|q| q.debit) {
        Some(Debit::Constant(k)) => format!("{k} KBPS"),
        Some(Debit::Variable(k)) => format!("VBR {k} KBPS"),
        None => "—".to_string(),
    };
    let frequence = match qualite {
        Some(q) => {
            let texte = format!("{:.1}", f64::from(q.frequence) / 1000.0);
            let texte = texte.strip_suffix(".0").unwrap_or(&texte).replace('.', ",");
            format!("{texte} KHZ")
        }
        None => "—".to_string(),
    };
    let egalise = match gain {
        None => "OFF".to_string(),
        Some(None) => "MESURE…".to_string(),
        Some(Some(facteur)) => texte_decibels(facteur),
    };
    let sortie = match sortie {
        Some(nom) => abreger(&nom.to_uppercase(), 18),
        None => "AUCUNE".to_string(),
    };
    vec![
        ("DÉBIT", debit),
        ("FRÉQ.", frequence),
        ("VOLUME ÉGAL.", egalise),
        ("SORTIE", sortie),
    ]
}

/// Un facteur de volume en décibels, à la française : « −4,8 DB », « +1,2 DB ».
fn texte_decibels(facteur: f32) -> String {
    let db = 20.0 * facteur.max(1e-6).log10();
    let signe = if db < -0.05 {
        "−"
    } else if db > 0.05 {
        "+"
    } else {
        ""
    };
    format!("{signe}{:.1} DB", db.abs()).replace('.', ",")
}

/// Coupe un texte trop long, avec « … » à la fin.
fn abreger(texte: &str, max: usize) -> String {
    if texte.chars().count() <= max {
        return texte.to_string();
    }
    let mut court: String = texte.chars().take(max - 1).collect();
    court.push('…');
    court
}

/// Dessine le bloc sur toute la largeur. Un clic dessus ouvre les infos du
/// fichier (comme la petite ligne sous la pochette d'avant).
pub fn afficher(ui: &mut egui::Ui, bloc: &EnLecture) -> egui::Response {
    let (rect, reponse) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HAUTEUR),
        egui::Sense::click(),
    );
    if !ui.is_rect_visible(rect) {
        return reponse;
    }
    let peintre = ui.painter_at(rect.expand(12.0));
    let coins = Coins::diagonale(18.0);
    // Le fond : un dégradé discret du cyan (à gauche) vers le magenta (à droite).
    theme::cadre(&peintre, rect, coins, PANNEAU, Stroke::NONE, 0.0);
    let points = theme::contour(rect, coins);
    // Deux dégradés superposés : cyan qui s'éteint vers la droite, magenta
    // qui s'allume ; le milieu reste presque de la couleur du panneau.
    let rien = Color32::TRANSPARENT;
    let cyan = theme::attenue(CYAN, 34);
    let magenta = theme::attenue(MAGENTA, 30);
    theme::degrade(&peintre, &points, cyan, rien, egui::Vec2::X);
    theme::degrade(&peintre, &points, rien, magenta, egui::Vec2::X);
    let survol = reponse.hovered();
    let bord = theme::attenue(CYAN, if survol { 200 } else { 110 });
    peintre.add(egui::Shape::closed_line(points, Stroke::new(1.0_f32, bord)));
    // Le petit trait lumineux sur l'angle coupé en haut à gauche.
    peintre.line_segment(
        [
            egui::pos2(rect.left() - 2.0, rect.top() + 20.0),
            egui::pos2(rect.left() + 20.0, rect.top() - 2.0),
        ],
        Stroke::new(2.0_f32, CYAN),
    );

    let interieur = rect.shrink2(egui::vec2(18.0, 14.0));
    let infos_visibles = interieur.width() > 520.0;
    let largeur_texte = if infos_visibles {
        interieur.width() - LARGEUR_INFOS - 18.0
    } else {
        interieur.width()
    };

    // L'état : voyant jaune et « EN LECTURE · TITRE 3 / 12 ».
    let mono = egui::FontId::monospace(12.0);
    let mut y = interieur.top() + 7.0;
    let voyant = egui::pos2(interieur.left() + 4.0, y);
    if bloc.voyant {
        theme::halo(
            &peintre,
            Rect::from_center_size(voyant, egui::vec2(8.0, 8.0)),
            JAUNE,
            0.3 + 0.6 * theme::pulsation(),
        );
        peintre.circle_filled(
            voyant,
            4.0,
            theme::fondu(JAUNE, 0.6 + 0.4 * theme::pulsation()),
        );
    } else {
        peintre.circle_stroke(voyant, 3.5, Stroke::new(1.0_f32, theme::fondu(JAUNE, 0.6)));
    }
    let mut ligne_etat = bloc.etat.to_uppercase();
    if let Some(compteur) = &bloc.compteur {
        ligne_etat.push_str(" · ");
        ligne_etat.push_str(&compteur.to_uppercase());
    }
    peintre.text(
        egui::pos2(interieur.left() + 16.0, y),
        egui::Align2::LEFT_CENTER,
        ligne_etat,
        mono.clone(),
        JAUNE,
    );

    // Le titre, en grand et lumineux, coupé s'il est trop long.
    y += 10.0;
    let titre_police = crate::police::titres(ui.ctx(), 28.0);
    let titre = ajuster(
        ui,
        &bloc.titre.to_uppercase(),
        titre_police.clone(),
        1.0,
        largeur_texte,
    );
    theme::texte_lumineux(
        &peintre,
        egui::pos2(interieur.left(), y),
        egui::Align2::LEFT_TOP,
        &titre,
        titre_police,
        1.0,
        TEXTE,
        CYAN,
    );
    y += 36.0;
    if let Some(artiste) = &bloc.artiste {
        let police = egui::FontId::proportional(19.0);
        let artiste = ajuster(
            ui,
            &artiste.to_uppercase(),
            police.clone(),
            1.5,
            largeur_texte,
        );
        theme::texte_lumineux(
            &peintre,
            egui::pos2(interieur.left(), y),
            egui::Align2::LEFT_TOP,
            &artiste,
            police,
            1.5,
            MAGENTA,
            MAGENTA,
        );
    }
    y += 24.0;
    if let Some(album) = &bloc.album {
        let police = egui::FontId::proportional(14.0);
        let album = ajuster(
            ui,
            &format!("Album · {album}"),
            police.clone(),
            0.0,
            largeur_texte,
        );
        peintre.text(
            egui::pos2(interieur.left(), y),
            egui::Align2::LEFT_TOP,
            album,
            police,
            TEXTE_DISCRET,
        );
    }

    // Les infos techniques, à droite : libellé discret, valeur cyan.
    if infos_visibles {
        let gauche = interieur.right() - LARGEUR_INFOS;
        let hauteur_ligne = 19.0;
        let debut = interieur.center().y - hauteur_ligne * bloc.techniques.len() as f32 / 2.0;
        for (n, (libelle, valeur)) in bloc.techniques.iter().enumerate() {
            let y = debut + hauteur_ligne * (n as f32 + 0.5);
            peintre.text(
                egui::pos2(gauche, y),
                egui::Align2::LEFT_CENTER,
                *libelle,
                mono.clone(),
                TEXTE_DISCRET,
            );
            peintre.text(
                egui::pos2(interieur.right(), y),
                egui::Align2::RIGHT_CENTER,
                valeur,
                mono.clone(),
                CYAN,
            );
        }
    }
    reponse
}

/// Raccourcit `texte` (avec « … ») pour qu'il tienne dans `largeur`.
fn ajuster(
    ui: &egui::Ui,
    texte: &str,
    police: egui::FontId,
    espacement: f32,
    largeur: f32,
) -> String {
    let mesure = |t: &str| {
        theme::mise_en_page(ui.ctx(), t, police.clone(), espacement)
            .size()
            .x
    };
    if mesure(texte) <= largeur {
        return texte.to_string();
    }
    let lettres: Vec<char> = texte.chars().collect();
    // On cherche la plus longue coupe qui tient (par dichotomie : les titres
    // sont courts, mais on mesure à chaque image).
    let (mut bas, mut haut) = (0, lettres.len());
    while bas < haut {
        let milieu = (bas + haut).div_ceil(2);
        let essai: String = lettres[..milieu].iter().collect::<String>() + "…";
        if mesure(&essai) <= largeur {
            bas = milieu;
        } else {
            haut = milieu - 1;
        }
    }
    lettres[..bas]
        .iter()
        .collect::<String>()
        .trim_end()
        .to_string()
        + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infos_fichier::{Canaux, Qualite};
    use std::time::Duration;

    fn infos(debit: Debit, frequence: u32) -> Infos {
        Infos {
            taille: 1,
            qualite: Some(Qualite {
                debit,
                frequence,
                canaux: Canaux::StereoJointe,
                version: "MPEG-1",
                duree: Duration::from_secs(1),
            }),
        }
    }

    #[test]
    fn les_infos_techniques_s_ecrivent_comme_dans_la_maquette() {
        let lignes = infos_techniques(
            Some(&infos(Debit::Constant(320), 44_100)),
            Some(Some(0.575)),
            Some("Casque USB"),
        );
        assert_eq!(
            lignes,
            vec![
                ("DÉBIT", "320 KBPS".to_string()),
                ("FRÉQ.", "44,1 KHZ".to_string()),
                ("VOLUME ÉGAL.", "−4,8 DB".to_string()),
                ("SORTIE", "CASQUE USB".to_string()),
            ]
        );
    }

    #[test]
    fn sans_infos_ni_sortie_on_le_dit() {
        let lignes = infos_techniques(None, None, None);
        assert_eq!(lignes[0].1, "—");
        assert_eq!(lignes[1].1, "—");
        assert_eq!(lignes[2].1, "OFF");
        assert_eq!(lignes[3].1, "AUCUNE");
        let lignes = infos_techniques(Some(&infos(Debit::Variable(245), 48_000)), Some(None), None);
        assert_eq!(lignes[0].1, "VBR 245 KBPS");
        assert_eq!(lignes[1].1, "48 KHZ");
        assert_eq!(lignes[2].1, "MESURE…");
    }

    #[test]
    fn les_decibels_ont_leur_signe() {
        assert_eq!(texte_decibels(1.0), "0,0 DB");
        assert_eq!(texte_decibels(2.0), "+6,0 DB");
        assert_eq!(texte_decibels(0.5), "−6,0 DB");
    }

    #[test]
    fn une_sortie_au_nom_tres_long_est_abregee() {
        let lignes = infos_techniques(
            None,
            None,
            Some("Haut-parleurs (Realtek High Definition Audio)"),
        );
        assert_eq!(lignes[3].1.chars().count(), 18);
        assert!(lignes[3].1.ends_with('…'));
    }

    #[test]
    fn un_titre_trop_long_est_coupe_pour_tenir() {
        let ctx = egui::Context::default();
        crate::police::installer(&ctx);
        let _ = ctx.run(Default::default(), |_| {});
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let police = egui::FontId::proportional(20.0);
                let court = ajuster(ui, "NIGHTCALL", police.clone(), 1.0, 1000.0);
                assert_eq!(court, "NIGHTCALL");
                let long = "UN TITRE BEAUCOUP TROP LONG POUR LA PLACE".repeat(3);
                let coupe = ajuster(ui, &long, police.clone(), 1.0, 200.0);
                assert!(coupe.ends_with('…'));
                let largeur = theme::mise_en_page(ui.ctx(), &coupe, police, 1.0).size().x;
                assert!(largeur <= 200.0);
            });
        });
    }
}
