//! Les paroles synchronisées : si un fichier .lrc porte le même nom que le
//! MP3 (chanson.mp3 → chanson.lrc), ses paroles défilent par-dessus la
//! pochette en même temps que la musique.
//!
//! Le .lrc peut être à côté du MP3 (mis là par l'utilisateur, ou créé par
//! une ancienne version du lecteur), ou dans notre dossier des paroles
//! (%APPDATA%\lecteur_mp3\paroles), qui reprend les sous-dossiers de la
//! Musique : ceux que le lecteur crée y sont rangés, pour ne pas encombrer
//! le dossier Musique.
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

/// Le fichier .lrc à côté d'un MP3 : même nom, même dossier.
pub fn fichier_lrc(mp3: &Path) -> PathBuf {
    mp3.with_extension("lrc")
}

/// Ce que le lecteur écrit dans l'en-tête des .lrc qu'il crée (voir
/// generer_paroles.rs) : c'est ainsi qu'on les reconnaît.
const MARQUE_DU_LECTEUR: &str = "[by:LRCLIB";

/// Où sont rangées les paroles créées par le lecteur.
pub struct Rangement {
    /// Notre dossier des paroles.
    pub dossier: PathBuf,
    /// Le dossier Musique, dont on reprend les sous-dossiers.
    pub musique: PathBuf,
}

impl Rangement {
    /// Le rangement du lecteur : %APPDATA%\lecteur_mp3\paroles sous Windows,
    /// ~/.config/lecteur_mp3/paroles sous Linux.
    pub fn du_lecteur() -> Option<Self> {
        Some(Self {
            dossier: crate::systeme::fichier_de_reglage("paroles")?,
            musique: crate::systeme::dossier_musique(),
        })
    }

    /// La place d'un .lrc dans notre dossier. Un morceau de la Musique garde
    /// ses sous-dossiers (Musique\Artiste\Album\01.mp3 →
    /// paroles\Artiste\Album\01.lrc), pour que deux morceaux du même nom ne
    /// se mélangent pas. Un morceau rangé ailleurs va dans « autres », avec
    /// tout son chemin (D:\Son\a.mp3 → paroles\autres\D\Son\a.lrc).
    pub fn place(&self, mp3: &Path) -> PathBuf {
        let relatif = match mp3.strip_prefix(&self.musique) {
            Ok(relatif) if !self.musique.as_os_str().is_empty() => relatif.to_path_buf(),
            _ => {
                let mut chemin = PathBuf::from("autres");
                for morceau in mp3.components() {
                    use std::path::Component;
                    match morceau {
                        // « C: » → « C », « \\serveur\partage » → « serveur_partage ».
                        Component::Prefix(p) => {
                            let nom: String = p
                                .as_os_str()
                                .to_string_lossy()
                                .chars()
                                .map(|c| if matches!(c, '\\' | '/') { '_' } else { c })
                                .filter(|c| !matches!(c, ':' | '?' | '.'))
                                .collect();
                            let nom = nom.trim_matches('_');
                            if !nom.is_empty() {
                                chemin.push(nom);
                            }
                        }
                        Component::Normal(nom) => chemin.push(nom),
                        // Jamais de « .. » : on ne sort pas de notre dossier.
                        _ => {}
                    }
                }
                chemin
            }
        };
        self.dossier.join(relatif).with_extension("lrc")
    }

    /// Le .lrc d'un MP3 : d'abord celui à côté du MP3 (souvent mis là par
    /// l'utilisateur), sinon celui de notre dossier.
    pub fn trouver(&self, mp3: &Path) -> Option<PathBuf> {
        [fichier_lrc(mp3), self.place(mp3)]
            .into_iter()
            .find(|f| f.is_file())
    }

    /// Range dans notre dossier les .lrc qu'une ancienne version du lecteur
    /// avait créés à côté des MP3. Ceux de l'utilisateur ne bougent pas.
    pub fn ranger_les_anciens(&self, morceaux: &[PathBuf]) -> Bilan {
        let mut bilan = Bilan::default();
        for mp3 in morceaux {
            let ancien = fichier_lrc(mp3);
            let Ok(octets) = std::fs::read(&ancien) else {
                continue;
            };
            if !decoder(&octets).contains(MARQUE_DU_LECTEUR) {
                continue;
            }
            let nouveau = self.place(mp3);
            let resultat = if nouveau.exists() {
                // Déjà rangé (copie d'avant) : on garde celui de notre dossier.
                std::fs::remove_file(&ancien)
            } else {
                deplacer(&ancien, &nouveau)
            };
            match resultat {
                Ok(()) => bilan.ranges += 1,
                Err(erreur) => bilan.erreurs.push((ancien, erreur.to_string())),
            }
        }
        bilan
    }
}

/// Le résultat du rangement des anciens .lrc.
#[derive(Debug, Default)]
pub struct Bilan {
    pub ranges: usize,
    pub erreurs: Vec<(PathBuf, String)>,
}

/// Déplace un fichier en créant les dossiers qui manquent. Si la Musique
/// n'est pas sur le même disque que notre dossier, Windows refuse de
/// simplement le déplacer : on le copie puis on efface l'original.
fn deplacer(de: &Path, vers: &Path) -> std::io::Result<()> {
    if let Some(dossier) = vers.parent() {
        std::fs::create_dir_all(dossier)?;
    }
    if std::fs::rename(de, vers).is_err() {
        std::fs::copy(de, vers)?;
        std::fs::remove_file(de)?;
    }
    Ok(())
}

/// Les paroles d'un MP3, si un .lrc l'accompagne, à côté de lui ou dans
/// notre dossier (et contient au moins une phrase avec son moment).
pub fn charger(mp3: &Path) -> Option<Paroles> {
    let fichier = match Rangement::du_lecteur() {
        Some(rangement) => rangement.trouver(mp3)?,
        None => fichier_lrc(mp3),
    };
    let octets = std::fs::read(fichier).ok()?;
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
        // La phrase en cours en Orbitron (police des titres néon), les
        // autres en texte courant.
        let (taille, taille_courante) = (16.0, 18.0);
        let mises_en_page: Vec<_> = self
            .lignes
            .iter()
            .enumerate()
            .map(|(n, (_, texte))| {
                let (police, couleur) = if Some(n) == courante {
                    (
                        crate::police::titres(ui.ctx(), taille_courante),
                        egui::Color32::WHITE,
                    )
                } else {
                    (
                        egui::FontId::proportional(taille),
                        egui::Color32::from_white_alpha(150),
                    )
                };
                // Une phrase vide (passage instrumental) garde sa place.
                let texte = if texte.is_empty() {
                    "♪"
                } else {
                    texte.as_str()
                };
                let mut mise_en_page =
                    egui::text::LayoutJob::simple(texte.to_string(), police, couleur, largeur);
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
        for (n, (galet, y)) in mises_en_page.into_iter().zip(hauts).enumerate() {
            let y = haut + y;
            if y > cadre.bottom() || y + galet.size().y < cadre.top() {
                continue;
            }
            // (Le texte centré part du milieu du cadre.)
            let position = egui::pos2(cadre.center().x, y);
            // La phrase en cours brille d'une lueur magenta, comme un néon.
            let force = crate::theme::intensite();
            if Some(n) == courante && force > 0.0 {
                let lueur = crate::theme::fondu(crate::theme::MAGENTA, 0.3 * force.min(1.5));
                let r = 2.0 * force.sqrt();
                for (dx, dy) in [(r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)] {
                    peintre.galley_with_override_text_color(
                        position + egui::vec2(dx, dy),
                        galet.clone(),
                        lueur,
                    );
                }
            }
            peintre.galley(position, galet, egui::Color32::WHITE);
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
        // Les polices néon (le titre « SPECTRE », la phrase en cours) en ont besoin.
        crate::police::installer(&ctx);
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

    #[test]
    fn place_dans_notre_dossier() {
        let rangement = Rangement {
            dossier: PathBuf::from("/config/lecteur_mp3/paroles"),
            musique: PathBuf::from("/maison/Musique"),
        };
        // Dans la Musique : mêmes sous-dossiers.
        assert_eq!(
            rangement.place(Path::new(
                "/maison/Musique/Daft Punk/Discovery/01 - Intro.mp3"
            )),
            Path::new("/config/lecteur_mp3/paroles/Daft Punk/Discovery/01 - Intro.lrc")
        );
        // Ailleurs : sous « autres », avec tout le chemin.
        assert_eq!(
            rangement.place(Path::new("/media/cle/Son/a.b.mp3")),
            Path::new("/config/lecteur_mp3/paroles/autres/media/cle/Son/a.b.lrc")
        );
        // Jamais en dehors de notre dossier.
        assert!(rangement
            .place(Path::new("/maison/../../etc/x.mp3"))
            .starts_with("/config/lecteur_mp3/paroles/autres"));
    }

    #[cfg(windows)]
    #[test]
    fn place_sous_windows() {
        let rangement = Rangement {
            dossier: PathBuf::from(r"C:\Users\Luce\AppData\Roaming\lecteur_mp3\paroles"),
            musique: PathBuf::from(r"C:\Users\Luce\Music"),
        };
        assert_eq!(
            rangement.place(Path::new(r"C:\Users\Luce\Music\Artiste\01.mp3")),
            rangement.dossier.join(r"Artiste\01.lrc")
        );
        assert_eq!(
            rangement.place(Path::new(r"D:\Son\a.mp3")),
            rangement.dossier.join(r"autres\D\Son\a.lrc")
        );
        assert_eq!(
            rangement.place(Path::new(r"\\nas\musique\a.mp3")),
            rangement.dossier.join(r"autres\nas_musique\a.lrc")
        );
    }

    #[test]
    fn trouve_et_range_les_anciens() {
        let dossier =
            std::env::temp_dir().join(format!("paroles_rangement_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dossier);
        let rangement = Rangement {
            dossier: dossier.join("paroles"),
            musique: dossier.join("Musique"),
        };
        let album = rangement.musique.join("Album");
        std::fs::create_dir_all(&album).unwrap();
        let a_nous = album.join("a nous.mp3");
        let a_elle = album.join("a elle.mp3");
        let en_double = album.join("en double.mp3");
        let sans = album.join("sans.mp3");
        let ecrire = |f: &Path, texte: &str| {
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, texte).unwrap();
        };
        ecrire(
            &fichier_lrc(&a_nous),
            "[ar:A]\n[by:LRCLIB : A — B]\n[00:01.00]la",
        );
        ecrire(&fichier_lrc(&a_elle), "[00:01.00]mes paroles");
        ecrire(&fichier_lrc(&en_double), "[by:LRCLIB : X]\n[00:01.00]vieux");
        ecrire(
            &rangement.place(&en_double),
            "[by:LRCLIB : X]\n[00:01.00]rangé",
        );

        // Celui à côté du MP3 passe en premier.
        assert_eq!(rangement.trouver(&en_double), Some(fichier_lrc(&en_double)));
        assert_eq!(rangement.trouver(&sans), None);

        let bilan = rangement.ranger_les_anciens(&[
            a_nous.clone(),
            a_elle.clone(),
            en_double.clone(),
            sans.clone(),
        ]);
        assert_eq!(bilan.ranges, 2);
        assert!(bilan.erreurs.is_empty());
        // Les nôtres ont quitté la Musique…
        assert!(!fichier_lrc(&a_nous).exists() && !fichier_lrc(&en_double).exists());
        assert_eq!(rangement.trouver(&a_nous), Some(rangement.place(&a_nous)));
        assert!(std::fs::read_to_string(rangement.place(&a_nous))
            .unwrap()
            .ends_with("[00:01.00]la"));
        // …sans écraser celui qui était déjà rangé…
        assert!(std::fs::read_to_string(rangement.place(&en_double))
            .unwrap()
            .ends_with("rangé"));
        // …et celui de l'utilisatrice n'a pas bougé.
        assert_eq!(rangement.trouver(&a_elle), Some(fichier_lrc(&a_elle)));
        std::fs::remove_dir_all(&dossier).unwrap();
    }
}
