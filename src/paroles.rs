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
    /// en cours au milieu, plus grande ; les autres plus discrètes autour.
    /// Le texte glisse doucement d'une phrase à l'autre.
    pub fn afficher(&self, ui: &egui::Ui, cadre: egui::Rect, position: f32, teintes: Teintes) {
        let peintre = ui.painter_at(cadre);
        // Un voile sur la pochette, pour que le texte se lise bien.
        peintre.rect_filled(cadre, 0.0, teintes.voile);

        let courante = self.ligne_courante(position);
        let largeur = cadre.width() - 32.0;
        let (taille, taille_courante) = (17.0, 21.0);
        let mises_en_page: Vec<_> = self
            .lignes
            .iter()
            .enumerate()
            .map(|(n, (_, texte))| {
                let (taille, couleur) = if Some(n) == courante {
                    (taille_courante, teintes.texte)
                } else {
                    (taille, teintes.autres)
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

/// Les couleurs des paroles, choisies d'après la pochette pour bien
/// ressortir : un voile posé sur la pochette, la couleur de la phrase en
/// cours et celle des autres phrases.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Teintes {
    pub voile: egui::Color32,
    pub texte: egui::Color32,
    pub autres: egui::Color32,
}

impl Teintes {
    /// Sans pochette : texte blanc sur fond noir (rien à voiler).
    pub const SANS_POCHETTE: Self = Self {
        voile: egui::Color32::TRANSPARENT,
        texte: egui::Color32::WHITE,
        autres: egui::Color32::from_gray(140),
    };

    /// Les couleurs pour une pochette dont la couleur moyenne est `moyenne`.
    /// Pochette sombre : voile noir et texte très clair ; pochette claire :
    /// voile blanc et texte très foncé. Le texte garde un peu de la couleur
    /// de la pochette, pour aller avec elle.
    pub fn pour(moyenne: egui::Color32) -> Self {
        let (extreme, voile) = if luminance(moyenne) > SEUIL_CLAIR {
            (egui::Color32::BLACK, egui::Color32::from_white_alpha(150))
        } else {
            (egui::Color32::WHITE, egui::Color32::from_black_alpha(150))
        };
        let fond = melanger(moyenne, opaque(voile), voile.a() as f32 / 255.0);
        let texte = melanger(moyenne, extreme, 0.88);
        Self {
            voile,
            texte,
            autres: melanger(fond, texte, 0.6),
        }
    }
}

/// Au-dessus de cette luminance (0 = noir, 1 = blanc), une couleur se lit
/// mieux avec du texte noir qu'avec du texte blanc.
const SEUIL_CLAIR: f32 = 0.18;

/// La couleur moyenne d'une image (on n'en regarde qu'un point sur
/// plusieurs : c'est bien assez et beaucoup plus rapide).
pub fn couleur_moyenne(image: &egui::ColorImage) -> egui::Color32 {
    let pas = (image.pixels.len() / 10_000).max(1);
    let (mut r, mut v, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for pixel in image.pixels.iter().step_by(pas) {
        r += pixel.r() as u64;
        v += pixel.g() as u64;
        b += pixel.b() as u64;
        n += 1;
    }
    if n == 0 {
        return egui::Color32::BLACK;
    }
    egui::Color32::from_rgb((r / n) as u8, (v / n) as u8, (b / n) as u8)
}

/// La luminance d'une couleur telle que l'œil la perçoit, de 0 (noir) à 1
/// (blanc). Le vert paraît bien plus clair que le bleu.
fn luminance(couleur: egui::Color32) -> f32 {
    let lineaire = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lineaire(couleur.r()) + 0.7152 * lineaire(couleur.g()) + 0.0722 * lineaire(couleur.b())
}

/// `a` mélangé à `b` : 0 = que `a`, 1 = que `b`.
fn melanger(a: egui::Color32, b: egui::Color32, part_de_b: f32) -> egui::Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * part_de_b).round() as u8;
    egui::Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// La couleur pleine d'un voile (egui range ses couleurs transparentes
/// « prémultipliées »).
fn opaque(voile: egui::Color32) -> egui::Color32 {
    let [r, v, b, _] = voile.to_srgba_unmultiplied();
    egui::Color32::from_rgb(r, v, b)
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
                    paroles.afficher(ui, cadre, position, Teintes::SANS_POCHETTE);
                });
            });
        }
    }

    /// Le rapport de contraste entre deux couleurs (1 = aucun, 21 = noir
    /// sur blanc). On vise au moins 4,5 pour du texte, comme sur le web.
    fn contraste(a: egui::Color32, b: egui::Color32) -> f32 {
        let (a, b) = (luminance(a) + 0.05, luminance(b) + 0.05);
        a.max(b) / a.min(b)
    }

    #[test]
    fn le_texte_ressort_sur_toutes_les_couleurs_de_pochette() {
        let niveaux = (0..=255).step_by(15).map(|n| n as u8);
        for r in niveaux.clone() {
            for v in niveaux.clone() {
                for b in niveaux.clone() {
                    let moyenne = egui::Color32::from_rgb(r, v, b);
                    let teintes = Teintes::pour(moyenne);
                    let a = teintes.voile.a() as f32 / 255.0;
                    let fond = melanger(moyenne, opaque(teintes.voile), a);
                    let (c, autres) = (
                        contraste(teintes.texte, fond),
                        contraste(teintes.autres, fond),
                    );
                    assert!(c >= 7.0, "{moyenne:?} : contraste {c}");
                    assert!(autres >= 3.0, "{moyenne:?} : autres phrases {autres}");
                }
            }
        }
    }

    #[test]
    fn pochette_claire_texte_fonce_pochette_sombre_texte_clair() {
        let claire = Teintes::pour(egui::Color32::from_rgb(240, 230, 200));
        assert!(luminance(claire.texte) < 0.05);
        let sombre = Teintes::pour(egui::Color32::from_rgb(30, 20, 60));
        assert!(luminance(sombre.texte) > 0.7);
        assert_eq!(Teintes::SANS_POCHETTE.texte, egui::Color32::WHITE);
    }

    #[test]
    fn couleur_moyenne_d_une_image() {
        let mut image = egui::ColorImage::new([10, 10], egui::Color32::from_rgb(200, 0, 0));
        for pixel in &mut image.pixels[50..] {
            *pixel = egui::Color32::from_rgb(0, 0, 100);
        }
        assert_eq!(couleur_moyenne(&image), egui::Color32::from_rgb(100, 0, 50));
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
