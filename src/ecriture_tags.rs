//! L'écriture des tags ID3 dans les fichiers MP3 (titre, artiste, album,
//! pochette…), avec la crate `id3` : symphonia sait seulement les lire.
//!
//! On ne touche jamais directement au fichier : on écrit dans une copie,
//! on vérifie que la musique y est restée identique octet pour octet, et
//! seulement alors la copie remplace l'original. Si quoi que ce soit
//! échoue, l'original reste tel qu'il était.
//!
//! `modifier` sert à n'importe quel changement de tags (par exemple, plus
//! tard, le volume égalisé « ReplayGain ») ; `enregistrer` est ce que fait
//! la fenêtre « Modifier les tags ».

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use id3::{Tag, TagLike, Version};

/// Un champ de texte qu'on peut modifier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Champ {
    Titre,
    Artiste,
    Album,
    ArtisteAlbum,
    /// Le numéro de la piste sur l'album (un nombre).
    Numero,
}

/// Ce qu'on fait de la pochette.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Pochette {
    /// On n'y touche pas.
    #[default]
    Garder,
    /// Une nouvelle image (le contenu d'un fichier JPEG ou PNG).
    Remplacer(Vec<u8>),
    /// Plus de pochette du tout.
    Retirer,
}

/// Les changements demandés. Seuls les champs listés sont modifiés : les
/// autres tags du fichier (année, genre, paroles…) restent tels quels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Modifications {
    /// Chaque champ changé et sa nouvelle valeur ; vide = on l'efface.
    pub textes: Vec<(Champ, String)>,
    pub pochette: Pochette,
}

/// Pourquoi l'enregistrement n'a pas pu se faire. Dans tous les cas, le
/// fichier d'origine n'a pas été modifié.
#[derive(Debug)]
pub enum Erreur {
    /// Les tags ID3 ne s'écrivent que dans les MP3 (pas les FLAC, WAV, OGG).
    PasUnMp3,
    LectureSeule,
    Introuvable,
    /// Les tags du fichier sont abîmés : on ne les réécrit pas, de peur de
    /// perdre ce qu'on n'a pas su lire.
    TagsIllisibles(String),
    /// Le numéro de piste n'est pas un nombre.
    NumeroInvalide(String),
    /// L'image choisie n'est ni un JPEG ni un PNG lisible.
    ImageInvalide,
    /// La copie ne contient plus exactement la même musique.
    VerificationRatee,
    /// La copie est prête mais n'a pas pu remplacer l'original (souvent :
    /// il est ouvert dans un autre programme).
    Remplacement(io::Error),
    Disque(io::Error),
}

impl fmt::Display for Erreur {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Erreur::PasUnMp3 => write!(f, "seuls les tags des MP3 se modifient pour l'instant"),
            Erreur::LectureSeule => write!(f, "le fichier est en lecture seule"),
            Erreur::Introuvable => write!(f, "le fichier est introuvable"),
            Erreur::TagsIllisibles(e) => write!(f, "ses tags actuels sont abîmés ({e})"),
            Erreur::NumeroInvalide(texte) => {
                write!(
                    f,
                    "« {texte} » n'est pas un numéro de piste (il faut un nombre)"
                )
            }
            Erreur::ImageInvalide => write!(f, "l'image n'est pas un JPEG ou un PNG lisible"),
            Erreur::VerificationRatee => {
                write!(
                    f,
                    "la vérification de la copie a échoué, l'original n'a pas été touché"
                )
            }
            Erreur::Remplacement(e) => write!(
                f,
                "impossible de remplacer le fichier, est-il ouvert dans un autre programme ? ({e})"
            ),
            Erreur::Disque(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for Erreur {
    fn from(e: io::Error) -> Self {
        Erreur::Disque(e)
    }
}

/// Applique les changements de la fenêtre « Modifier les tags ».
pub fn enregistrer(chemin: &Path, modifications: &Modifications) -> Result<(), Erreur> {
    // On vérifie tout avant de toucher au disque.
    let numero = match modifications
        .textes
        .iter()
        .find(|(c, _)| *c == Champ::Numero)
    {
        Some((_, texte)) if !texte.trim().is_empty() => {
            Some(Some(texte.trim().parse::<u32>().map_err(|_| {
                Erreur::NumeroInvalide(texte.trim().to_string())
            })?))
        }
        Some(_) => Some(None),
        None => None,
    };
    let image = match &modifications.pochette {
        Pochette::Remplacer(donnees) => Some((type_d_image(donnees)?, donnees.clone())),
        _ => None,
    };
    modifier(chemin, |tag| {
        for (champ, texte) in &modifications.textes {
            let texte = texte.trim();
            let cle = match champ {
                Champ::Titre => "TIT2",
                Champ::Artiste => "TPE1",
                Champ::Album => "TALB",
                Champ::ArtisteAlbum => "TPE2",
                Champ::Numero => continue,
            };
            if texte.is_empty() {
                tag.remove(cle);
            } else {
                tag.set_text(cle, texte);
            }
        }
        match numero {
            // `set_track` garde le nombre total de pistes (« 3/12 »), s'il y était.
            Some(Some(n)) => tag.set_track(n),
            Some(None) => tag.remove_track(),
            None => {}
        }
        match (&modifications.pochette, image) {
            (Pochette::Retirer, _) => tag.remove_all_pictures(),
            (Pochette::Remplacer(_), Some((mime, donnees))) => {
                // La nouvelle remplace la pochette « devant » (celle qu'on affiche).
                tag.remove_picture_by_type(id3::frame::PictureType::CoverFront);
                tag.add_frame(id3::frame::Picture {
                    mime_type: mime.to_string(),
                    picture_type: id3::frame::PictureType::CoverFront,
                    description: String::new(),
                    data: donnees,
                });
            }
            _ => {}
        }
    })
}

/// Le type d'une image, d'après son contenu. Seuls le JPEG et le PNG sont
/// acceptés : ce sont ceux que tous les lecteurs savent afficher.
fn type_d_image(donnees: &[u8]) -> Result<&'static str, Erreur> {
    let mime = match image::guess_format(donnees) {
        Ok(image::ImageFormat::Jpeg) => "image/jpeg",
        Ok(image::ImageFormat::Png) => "image/png",
        _ => return Err(Erreur::ImageInvalide),
    };
    image::load_from_memory(donnees).map_err(|_| Erreur::ImageInvalide)?;
    Ok(mime)
}

/// Lit les tags du fichier, laisse `changer` les modifier, et les réécrit
/// sans risque pour le fichier (voir le haut de ce module).
pub fn modifier(chemin: &Path, changer: impl FnOnce(&mut Tag)) -> Result<(), Erreur> {
    if !chemin
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
    {
        return Err(Erreur::PasUnMp3);
    }
    let infos = match fs::metadata(chemin) {
        Ok(infos) => infos,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Erreur::Introuvable),
        Err(e) => return Err(e.into()),
    };
    if infos.permissions().readonly() {
        return Err(Erreur::LectureSeule);
    }
    // On garde la version du fichier (2.3 ou 2.4). Un nouveau tag, ou un très
    // vieux (2.2), s'écrit en 2.3 : c'est celle que tous les logiciels lisent,
    // l'Explorateur Windows compris.
    let (mut tag, version) = match Tag::read_from_path(chemin) {
        Ok(tag) if tag.version() == Version::Id3v24 => (tag, Version::Id3v24),
        Ok(tag) => (tag, Version::Id3v23),
        Err(e) if matches!(e.kind, id3::ErrorKind::NoTag) => (Tag::new(), Version::Id3v23),
        Err(e) => return Err(Erreur::TagsIllisibles(e.to_string())),
    };
    changer(&mut tag);

    let copie = copie_de(chemin);
    let resultat = ecrire_dans_la_copie(chemin, &copie, &tag, version).and_then(|()| {
        // `rename` remplace l'original d'un coup, sous Windows comme sous Linux.
        fs::rename(&copie, chemin).map_err(Erreur::Remplacement)
    });
    if resultat.is_err() {
        let _ = fs::remove_file(&copie);
    }
    resultat
}

/// La copie de travail, à côté de l'original (sur le même disque, pour
/// pouvoir le remplacer d'un coup).
fn copie_de(chemin: &Path) -> PathBuf {
    let mut nom = chemin.file_name().unwrap_or_default().to_os_string();
    nom.push(".modif-tags.tmp");
    chemin.with_file_name(nom)
}

fn ecrire_dans_la_copie(
    original: &Path,
    copie: &Path,
    tag: &Tag,
    version: Version,
) -> Result<(), Erreur> {
    fs::copy(original, copie)?;
    tag.write_to_path(copie, version)
        .map_err(|e| Erreur::Disque(io::Error::other(e.to_string())))?;
    // On relit la copie : les tags doivent être ceux voulus…
    let relu = Tag::read_from_path(copie).map_err(|_| Erreur::VerificationRatee)?;
    if !memes_tags(&relu, tag) {
        return Err(Erreur::VerificationRatee);
    }
    // … et la musique, après les tags, exactement la même.
    if !meme_musique(original, copie)? {
        return Err(Erreur::VerificationRatee);
    }
    Ok(())
}

/// Les mêmes cadres de part et d'autre. Les cadres vides ne sont pas
/// écrits par `id3` : on ne les compte pas.
fn memes_tags(relu: &Tag, voulu: &Tag) -> bool {
    let pleins = |tag: &Tag| -> Vec<id3::Frame> {
        tag.frames()
            .filter(|cadre| cadre.content().text() != Some(""))
            .cloned()
            .collect()
    };
    let (relus, voulus) = (pleins(relu), pleins(voulu));
    relus.len() == voulus.len() && voulus.iter().all(|cadre| relus.contains(cadre))
}

/// La taille du tag ID3v2 au début d'un fichier (0 s'il n'y en a pas).
fn taille_du_tag(fichier: &mut impl Read) -> io::Result<u64> {
    let mut entete = [0u8; 10];
    match fichier.read_exact(&mut entete) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(0),
        Err(e) => return Err(e),
    }
    if &entete[..3] != b"ID3" {
        return Ok(0);
    }
    // La taille est écrite sur 4 octets de 7 bits chacun.
    let taille = entete[6..10]
        .iter()
        .fold(0u64, |total, &octet| (total << 7) | u64::from(octet & 0x7f));
    // Un « pied de page » de 10 octets suit parfois le tag.
    let pied = if entete[5] & 0x10 != 0 { 10 } else { 0 };
    Ok(10 + taille + pied)
}

/// Ce qui suit le tag ID3v2 (la musique) est-il identique dans les deux fichiers ?
fn meme_musique(a: &Path, b: &Path) -> io::Result<bool> {
    let ouvrir = |chemin: &Path| -> io::Result<BufReader<File>> {
        let mut fichier = File::open(chemin)?;
        let debut = taille_du_tag(&mut fichier)?;
        fichier.seek(SeekFrom::Start(debut))?;
        Ok(BufReader::new(fichier))
    };
    let (mut a, mut b) = (ouvrir(a)?, ouvrir(b)?);
    let (mut morceau_a, mut morceau_b) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let lus = lire_tout(&mut a, &mut morceau_a)?;
        if lus != lire_tout(&mut b, &mut morceau_b)? || morceau_a[..lus] != morceau_b[..lus] {
            return Ok(false);
        }
        if lus == 0 {
            return Ok(true);
        }
    }
}

/// Remplit `tampon` autant que possible (moins seulement à la fin du fichier).
fn lire_tout(source: &mut impl Read, tampon: &mut [u8]) -> io::Result<usize> {
    let mut lus = 0;
    while lus < tampon.len() {
        match source.read(&mut tampon[lus..])? {
            0 => break,
            n => lus += n,
        }
    }
    Ok(lus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::tests::{ecrire_mp3, ecrire_mp3_avec};
    use crate::tags::{lire, lire_pochette};

    fn textes(liste: &[(Champ, &str)]) -> Modifications {
        Modifications {
            textes: liste.iter().map(|(c, t)| (*c, t.to_string())).collect(),
            pochette: Pochette::Garder,
        }
    }

    fn jpeg_bleu() -> Vec<u8> {
        let image = image::RgbImage::from_pixel(6, 3, image::Rgb([0, 0, 255]));
        let mut jpeg = io::Cursor::new(Vec::new());
        image.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        jpeg.into_inner()
    }

    /// Tout ce qui suit le tag : la musique.
    fn musique(chemin: &Path) -> Vec<u8> {
        let tout = fs::read(chemin).unwrap();
        let debut = taille_du_tag(&mut tout.as_slice()).unwrap() as usize;
        tout[debut..].to_vec()
    }

    #[test]
    fn change_titre_artiste_album_et_garde_la_musique() {
        let chemin = ecrire_mp3(
            "ecrire base.mp3",
            Some("Vieux titre"),
            Some("Daft"),
            None,
            false,
            40,
        );
        let avant = musique(&chemin);
        let duree = lire(&chemin).duree;
        enregistrer(
            &chemin,
            &textes(&[
                (Champ::Titre, "One More Time"),
                (Champ::Artiste, "Daft Punk"),
                (Champ::Album, "Discovery — édition spéciale"),
                (Champ::ArtisteAlbum, "Daft Punk"),
                (Champ::Numero, "1"),
            ]),
        )
        .unwrap();
        let tags = lire(&chemin);
        assert_eq!(tags.titre.as_deref(), Some("One More Time"));
        assert_eq!(tags.artiste.as_deref(), Some("Daft Punk"));
        assert_eq!(tags.album.as_deref(), Some("Discovery — édition spéciale"));
        assert_eq!(tags.artiste_album.as_deref(), Some("Daft Punk"));
        assert_eq!(tags.numero, Some(1));
        assert_eq!(tags.duree, duree);
        assert_eq!(musique(&chemin), avant);
        assert!(!copie_de(&chemin).exists());
    }

    #[test]
    fn un_champ_vide_efface_le_tag_et_les_autres_restent() {
        let chemin = ecrire_mp3_avec(
            "ecrire effacer.mp3",
            &[
                ("TIT2", "Titre"),
                ("TPE1", "Artiste"),
                ("TCON", "House"),
                ("TRCK", "3/12"),
            ],
            true,
            20,
        );
        enregistrer(&chemin, &textes(&[(Champ::Artiste, "  ")])).unwrap();
        let tags = lire(&chemin);
        assert_eq!(tags.titre.as_deref(), Some("Titre"));
        assert_eq!(tags.artiste, None);
        assert_eq!(tags.numero, Some(3));
        // Ce que la fenêtre ne montre pas (le genre, la pochette) ne bouge pas.
        let tag = Tag::read_from_path(&chemin).unwrap();
        assert_eq!(tag.genre(), Some("House"));
        assert!(lire_pochette(&chemin).is_some());
    }

    #[test]
    fn le_numero_garde_le_total_de_pistes() {
        let chemin = ecrire_mp3_avec("ecrire numero.mp3", &[("TRCK", "3/12")], false, 20);
        enregistrer(&chemin, &textes(&[(Champ::Numero, "5")])).unwrap();
        let tag = Tag::read_from_path(&chemin).unwrap();
        assert_eq!(tag.track(), Some(5));
        assert_eq!(tag.total_tracks(), Some(12));
    }

    #[test]
    fn un_numero_qui_n_est_pas_un_nombre_ne_touche_a_rien() {
        let chemin = ecrire_mp3(
            "ecrire numero faux.mp3",
            Some("Titre"),
            None,
            None,
            false,
            20,
        );
        let avant = fs::read(&chemin).unwrap();
        let erreur = enregistrer(
            &chemin,
            &textes(&[(Champ::Titre, "Autre"), (Champ::Numero, "trois")]),
        );
        assert!(matches!(erreur, Err(Erreur::NumeroInvalide(_))));
        assert_eq!(fs::read(&chemin).unwrap(), avant);
    }

    #[test]
    fn ecrit_des_tags_dans_un_fichier_qui_n_en_a_pas() {
        let chemin = ecrire_mp3("ecrire sans tags.mp3", None, None, None, false, 30);
        let avant = fs::read(&chemin).unwrap();
        enregistrer(&chemin, &textes(&[(Champ::Titre, "Nouveau café ♫")])).unwrap();
        assert_eq!(lire(&chemin).titre.as_deref(), Some("Nouveau café ♫"));
        assert_eq!(musique(&chemin), avant);
        // Un nouveau tag s'écrit en ID3v2.3.
        assert_eq!(
            Tag::read_from_path(&chemin).unwrap().version(),
            Version::Id3v23
        );
    }

    #[test]
    fn garde_la_version_2_4() {
        let chemin = ecrire_mp3("ecrire v24.mp3", Some("Titre"), None, None, false, 20);
        enregistrer(&chemin, &textes(&[(Champ::Titre, "Été")])).unwrap();
        assert_eq!(
            Tag::read_from_path(&chemin).unwrap().version(),
            Version::Id3v24
        );
        assert_eq!(lire(&chemin).titre.as_deref(), Some("Été"));
    }

    #[test]
    fn remplace_puis_retire_la_pochette() {
        let chemin = ecrire_mp3("ecrire pochette.mp3", Some("Titre"), None, None, true, 20);
        let modifs = Modifications {
            pochette: Pochette::Remplacer(jpeg_bleu()),
            ..Default::default()
        };
        enregistrer(&chemin, &modifs).unwrap();
        let pochette = lire_pochette(&chemin).unwrap();
        assert_eq!(pochette.size, [6, 3]);
        let tag = Tag::read_from_path(&chemin).unwrap();
        assert_eq!(tag.pictures().count(), 1);
        assert_eq!(tag.pictures().next().unwrap().mime_type, "image/jpeg");

        let modifs = Modifications {
            pochette: Pochette::Retirer,
            ..Default::default()
        };
        enregistrer(&chemin, &modifs).unwrap();
        assert!(lire_pochette(&chemin).is_none());
        assert_eq!(lire(&chemin).titre.as_deref(), Some("Titre"));
    }

    #[test]
    fn refuse_une_image_qui_n_en_est_pas_une() {
        let chemin = ecrire_mp3(
            "ecrire image fausse.mp3",
            Some("Titre"),
            None,
            None,
            true,
            20,
        );
        let avant = fs::read(&chemin).unwrap();
        let modifs = Modifications {
            pochette: Pochette::Remplacer(b"pas une image".to_vec()),
            ..Default::default()
        };
        assert!(matches!(
            enregistrer(&chemin, &modifs),
            Err(Erreur::ImageInvalide)
        ));
        assert_eq!(fs::read(&chemin).unwrap(), avant);
    }

    /// Les tags ID3 ne vont que dans les MP3 : un FLAC ou un OGG n'est pas
    /// touché (son volume égalisé ne peut pas non plus y être écrit).
    #[test]
    fn un_fichier_qui_n_est_pas_un_mp3_n_est_pas_touche() {
        let dossier = std::env::temp_dir().join("dollwave_tests_tags");
        fs::create_dir_all(&dossier).unwrap();
        let chemin = dossier.join("ecrire pas un mp3.flac");
        fs::write(&chemin, include_bytes!("../tests/glissando.flac")).unwrap();
        let avant = fs::read(&chemin).unwrap();
        let erreur = enregistrer(&chemin, &textes(&[(Champ::Titre, "Autre")]));
        assert!(matches!(erreur, Err(Erreur::PasUnMp3)));
        assert_eq!(fs::read(&chemin).unwrap(), avant);
        assert!(!copie_de(&chemin).exists());
    }

    #[test]
    fn un_fichier_en_lecture_seule_n_est_pas_touche() {
        let chemin = ecrire_mp3(
            "ecrire lecture seule.mp3",
            Some("Titre"),
            None,
            None,
            false,
            20,
        );
        let mut droits = fs::metadata(&chemin).unwrap().permissions();
        droits.set_readonly(true);
        fs::set_permissions(&chemin, droits.clone()).unwrap();
        let avant = fs::read(&chemin).unwrap();
        let erreur = enregistrer(&chemin, &textes(&[(Champ::Titre, "Autre")]));
        assert!(matches!(erreur, Err(Erreur::LectureSeule)));
        assert_eq!(fs::read(&chemin).unwrap(), avant);
        assert!(!copie_de(&chemin).exists());
        // Pour que le test suivant puisse réécrire le fichier.
        #[allow(clippy::permissions_set_readonly_false)]
        droits.set_readonly(false);
        fs::set_permissions(&chemin, droits).unwrap();
    }

    #[test]
    fn un_fichier_introuvable() {
        let chemin = std::env::temp_dir().join("dollwave_tests_tags/n existe pas.mp3");
        let erreur = enregistrer(&chemin, &textes(&[(Champ::Titre, "Autre")]));
        assert!(matches!(erreur, Err(Erreur::Introuvable)));
    }

    #[test]
    fn la_verification_repere_une_musique_changee() {
        let a = ecrire_mp3("verif a.mp3", Some("A"), None, None, false, 20);
        let b = ecrire_mp3(
            "verif b.mp3",
            Some("Un titre plus long"),
            None,
            None,
            false,
            20,
        );
        assert!(meme_musique(&a, &b).unwrap());
        let c = ecrire_mp3("verif c.mp3", Some("A"), None, None, false, 21);
        assert!(!meme_musique(&a, &c).unwrap());
    }

    #[test]
    fn apres_l_enregistrement_les_nouveaux_tags_s_affichent() {
        use crate::tags::TagsConnus;
        use std::time::{Duration, Instant};
        let chemin = ecrire_mp3("ecrire relire.mp3", Some("Avant"), None, None, false, 20);
        let connus = TagsConnus::new(&eframe::egui::Context::default());
        let attendre = |voulu: &str| {
            let debut = Instant::now();
            while connus.libelle(&chemin) != voulu {
                assert!(
                    debut.elapsed() < Duration::from_secs(5),
                    "jamais « {voulu} »"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        attendre("Avant");
        let version = connus.version();
        enregistrer(&chemin, &textes(&[(Champ::Titre, "Après")])).unwrap();
        connus.relire(&chemin);
        attendre("Après");
        // La version change : la recherche et les vues Artistes/Albums se refont.
        let debut = Instant::now();
        while connus.version() == version {
            assert!(debut.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
