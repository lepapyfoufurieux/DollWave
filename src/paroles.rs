//! Les paroles synchronisées : si un fichier .lrc porte le même nom que le
//! MP3 (chanson.mp3 → chanson.lrc, dans le même dossier), ses paroles
//! défilent par-dessus la pochette en même temps que la musique.
//!
//! Le format .lrc est un simple fichier texte, une ligne par phrase chantée,
//! précédée du moment où elle commence :
//!
//! ```text
//! [ti:Titre]
//! [offset:+200]
//! [00:12.30]Première phrase
//! [00:15.80][01:20.00]Un refrain chanté deux fois
//! ```

use std::path::{Path, PathBuf};

use eframe::egui;

/// Les phrases d'une chanson, dans l'ordre où elles sont chantées.
#[derive(Debug, Default, PartialEq)]
pub struct Paroles {
    /// Le moment où chaque phrase commence (en secondes), et son texte.
    lignes: Vec<(f32, String)>,
}

/// Le fichier .lrc qui va avec un MP3 : même nom, même dossier.
pub fn fichier_lrc(mp3: &Path) -> PathBuf {
    mp3.with_extension("lrc")
}

/// Les paroles d'un MP3, si un .lrc l'accompagne (et contient au moins une
/// phrase avec son moment).
pub fn charger(mp3: &Path) -> Option<Paroles> {
    let octets = std::fs::read(fichier_lrc(mp3)).ok()?;
    let paroles = Paroles::lire(&decoder(&octets));
    (!paroles.lignes.is_empty()).then_some(paroles)
}

/// Le texte d'un fichier .lrc. La plupart sont en UTF-8, mais les anciens
/// fichiers faits sous Windows sont souvent en Windows-1252 (Latin-1) : sans
/// ce secours, « é » s'afficherait « � ».
pub fn decoder(octets: &[u8]) -> String {
    let octets = octets.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(octets);
    match std::str::from_utf8(octets) {
        Ok(texte) => texte.to_string(),
        Err(_) => octets.iter().map(|&o| windows_1252(o)).collect(),
    }
}

/// Un caractère Windows-1252. C'est le Latin-1, sauf de 0x80 à 0x9F où
/// Windows a rangé €, œ, les guillemets « courbes »…
fn windows_1252(octet: u8) -> char {
    const DE_80_A_9F: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match octet {
        0x80..=0x9F => DE_80_A_9F[(octet - 0x80) as usize],
        _ => octet as char,
    }
}

impl Paroles {
    /// Lit le texte d'un fichier .lrc. Les lignes sans moment (titre,
    /// artiste…) sont ignorées, sauf le décalage `[offset:±ms]`.
    pub fn lire(texte: &str) -> Self {
        let mut lignes = Vec::new();
        // Décalage en millisecondes : positif = les paroles arrivent plus tôt.
        let mut decalage = 0.0_f32;
        for ligne in texte.lines() {
            let mut reste = ligne.trim();
            let mut moments = Vec::new();
            // Une ligne peut commencer par plusieurs moments : [00:15.80][01:20.00]…
            while let Some(apres) = reste.strip_prefix('[') {
                let Some(fin) = apres.find(']') else { break };
                let etiquette = &apres[..fin];
                if let Some(moment) = lire_moment(etiquette) {
                    moments.push(moment);
                } else if let Some(valeur) = etiquette.strip_prefix("offset:") {
                    decalage = valeur.trim().parse().unwrap_or(0.0);
                }
                reste = &apres[fin + 1..];
            }
            if moments.is_empty() {
                continue;
            }
            let texte = sans_moments_des_mots(reste.trim());
            for moment in moments {
                lignes.push((moment, texte.clone()));
            }
        }
        for (moment, _) in &mut lignes {
            *moment = (*moment - decalage / 1000.0).max(0.0);
        }
        // Les refrains notés une seule fois avec plusieurs moments se
        // retrouvent à leur place (le tri garde l'ordre des phrases au même moment).
        lignes.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { lignes }
    }

    /// La phrase chantée à `position` (en secondes) : la dernière qui a
    /// commencé. Rien avant la première.
    pub fn ligne_courante(&self, position: f32) -> Option<usize> {
        self.lignes
            .partition_point(|(moment, _)| *moment <= position)
            .checked_sub(1)
    }

    /// Dessine les paroles dans `cadre` (par-dessus la pochette), la phrase
    /// en cours au milieu, en blanc ; les autres en gris autour. Le texte
    /// glisse doucement d'une phrase à l'autre.
    pub fn afficher(&self, ui: &egui::Ui, cadre: egui::Rect, position: f32) {
        let peintre = ui.painter_at(cadre);
        // La pochette est assombrie pour que le texte se lise bien.
        peintre.rect_filled(cadre, 0.0, egui::Color32::from_black_alpha(170));

        let courante = self.ligne_courante(position);
        let largeur = cadre.width() - 32.0;
        let (taille, taille_courante) = (17.0, 21.0);
        let mises_en_page: Vec<_> = self
            .lignes
            .iter()
            .enumerate()
            .map(|(n, (_, texte))| {
                let (taille, couleur) = if Some(n) == courante {
                    (taille_courante, egui::Color32::WHITE)
                } else {
                    (taille, egui::Color32::from_gray(150))
                };
                // Une phrase vide (passage instrumental) garde sa place.
                let texte = if texte.is_empty() {
                    "♪"
                } else {
                    texte.as_str()
                };
                let mut mise_en_page = egui::text::LayoutJob::simple(
                    texte.to_string(),
                    egui::FontId::proportional(taille),
                    couleur,
                    largeur,
                );
                mise_en_page.halign = egui::Align::Center;
                ui.fonts(|f| f.layout_job(mise_en_page))
            })
            .collect();

        // Où commence chaque phrase, en partant du haut de la première.
        let espace = 10.0;
        let mut hauts = Vec::with_capacity(mises_en_page.len());
        let mut y = 0.0;
        for galet in &mises_en_page {
            hauts.push(y);
            y += galet.size().y + espace;
        }
        // On vise le milieu de la phrase en cours (avant la première : la
        // première arrive par le bas).
        let cible = match courante {
            Some(n) => hauts[n] + mises_en_page[n].size().y / 2.0,
            None => -cadre.height() / 4.0,
        };
        let defilement =
            ui.ctx()
                .animate_value_with_time(ui.id().with("defilement_paroles"), cible, 0.3);

        let haut = cadre.center().y - defilement;
        for (galet, y) in mises_en_page.into_iter().zip(hauts) {
            let y = haut + y;
            if y > cadre.bottom() || y + galet.size().y < cadre.top() {
                continue;
            }
            // (Le texte centré part du milieu du cadre.)
            peintre.galley(egui::pos2(cadre.center().x, y), galet, egui::Color32::WHITE);
        }
    }
}

/// Un moment « mm:ss », « mm:ss.cc » (ou « mm:ss:cc »), en secondes. Rien
/// si l'étiquette n'en est pas un (ti:, ar:…).
fn lire_moment(etiquette: &str) -> Option<f32> {
    let (minutes, reste) = etiquette.split_once(':')?;
    let minutes: u32 = minutes.trim().parse().ok()?;
    // Certains logiciels écrivent les centièmes après « : » au lieu de « . ».
    let secondes = match reste.split_once(':') {
        Some((s, c)) => format!("{s}.{c}"),
        None => reste.to_string(),
    };
    if !secondes.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let secondes: f32 = secondes.trim().parse().ok()?;
    Some(minutes as f32 * 60.0 + secondes)
}

/// Le LRC « amélioré » note aussi le moment de chaque mot : <00:12.50>.
/// On ne les affiche pas.
fn sans_moments_des_mots(texte: &str) -> String {
    let mut propre = String::with_capacity(texte.len());
    let mut reste = texte;
    while let Some(debut) = reste.find('<') {
        match reste[debut..].find('>') {
            Some(fin) if lire_moment(&reste[debut + 1..debut + fin]).is_some() => {
                propre.push_str(&reste[..debut]);
                reste = &reste[debut + fin + 1..];
            }
            _ => {
                propre.push_str(&reste[..=debut]);
                reste = &reste[debut + 1..];
            }
        }
    }
    propre.push_str(reste);
    // Des espaces doubles peuvent rester là où étaient les moments.
    propre.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn textes(paroles: &Paroles) -> Vec<(f32, &str)> {
        paroles
            .lignes
            .iter()
            .map(|(m, t)| (*m, t.as_str()))
            .collect()
    }

    #[test]
    fn lit_les_phrases_et_ignore_le_titre() {
        let paroles = Paroles::lire(
            "[ti:Ma chanson]\n[ar:Moi]\n\n[00:12.30]Première phrase\n[01:05.5] Deuxième  \n",
        );
        assert_eq!(
            textes(&paroles),
            [(12.3, "Première phrase"), (65.5, "Deuxième")]
        );
    }

    #[test]
    fn un_refrain_a_plusieurs_moments_est_range_a_chacun() {
        let paroles = Paroles::lire("[00:10.00][00:30.00]Refrain\n[00:20.00]Couplet");
        assert_eq!(
            textes(&paroles),
            [(10.0, "Refrain"), (20.0, "Couplet"), (30.0, "Refrain")]
        );
    }

    #[test]
    fn formats_de_moment() {
        assert_eq!(lire_moment("01:02"), Some(62.0));
        assert_eq!(lire_moment("01:02.5"), Some(62.5));
        assert_eq!(lire_moment("00:02.250"), Some(2.25));
        assert_eq!(lire_moment("00:02:50"), Some(2.5));
        assert_eq!(lire_moment("ti:Titre"), None);
        assert_eq!(lire_moment("offset:+200"), None);
    }

    #[test]
    fn le_decalage_avance_ou_retarde_les_paroles() {
        let plus_tot = Paroles::lire("[offset:+500]\n[00:10.00]A\n[00:00.20]B");
        assert_eq!(textes(&plus_tot), [(0.0, "B"), (9.5, "A")]);
        let plus_tard = Paroles::lire("[offset:-1000]\n[00:10.00]A");
        assert_eq!(textes(&plus_tard), [(11.0, "A")]);
    }

    #[test]
    fn les_phrases_vides_et_les_moments_des_mots() {
        let paroles = Paroles::lire("[00:01.00]\n[00:02.00]<00:02.00>Un <00:02.40>deux <x>");
        assert_eq!(textes(&paroles), [(1.0, ""), (2.0, "Un deux <x>")]);
    }

    #[test]
    fn la_ligne_courante_suit_la_position() {
        let paroles = Paroles::lire("[00:10.00]A\n[00:20.00]B\n[00:30.00]C");
        assert_eq!(paroles.ligne_courante(0.0), None);
        assert_eq!(paroles.ligne_courante(9.99), None);
        assert_eq!(paroles.ligne_courante(10.0), Some(0));
        assert_eq!(paroles.ligne_courante(25.0), Some(1));
        // Un saut en arrière (scratch) : on revient à la bonne phrase.
        assert_eq!(paroles.ligne_courante(12.0), Some(0));
        assert_eq!(paroles.ligne_courante(500.0), Some(2));
    }

    #[test]
    fn utf8_avec_ou_sans_bom_et_windows_1252() {
        assert_eq!(decoder("Été".as_bytes()), "Été");
        assert_eq!(decoder(b"\xEF\xBB\xBF\xC3\xA9t\xC3\xA9"), "été");
        // « Œuvre à 5 € » enregistré par un vieux logiciel Windows.
        assert_eq!(decoder(b"\x8Cuvre \xE0 5 \x80"), "Œuvre à 5 €");
    }

    #[test]
    fn s_affiche_sans_planter_avant_pendant_et_apres() {
        let paroles = Paroles::lire("[00:01.00]Une phrase assez longue pour passer à la ligne dans un petit cadre\n[00:02.00]\n[00:03.00]Fin");
        let ctx = egui::Context::default();
        for position in [0.0, 1.5, 2.5, 100.0] {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let cadre =
                        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 200.0));
                    paroles.afficher(ui, cadre, position);
                });
            });
        }
    }

    #[test]
    fn le_lrc_a_cote_du_mp3() {
        let dossier = std::env::temp_dir().join(format!("paroles_test_{}", std::process::id()));
        std::fs::create_dir_all(&dossier).unwrap();
        let mp3 = dossier.join("chanson.mp3");
        assert_eq!(fichier_lrc(&mp3), dossier.join("chanson.lrc"));
        assert_eq!(charger(&mp3), None);
        std::fs::write(dossier.join("chanson.lrc"), b"[00:01.00]D\xE9j\xE0").unwrap();
        assert_eq!(textes(&charger(&mp3).unwrap()), [(1.0, "Déjà")]);
        // Un .lrc sans aucun moment ne compte pas.
        std::fs::write(dossier.join("chanson.lrc"), "Juste du texte").unwrap();
        assert_eq!(charger(&mp3), None);
        std::fs::remove_dir_all(&dossier).unwrap();
    }
}
