//! Le lecteur s'inscrit auprès du système comme « lecteur média » : les
//! touches lecture, suivant et précédent du clavier lui arrivent alors même
//! quand il est en arrière-plan (ou rangé près de l'horloge).
//!
//! La bibliothèque souvlaki parle au système à notre place :
//! - Windows : les « contrôles de transport média » (SMTC), les mêmes qui
//!   affichent le petit panneau près du volume ;
//! - Linux : MPRIS, que les bureaux (GNOME, KDE…) utilisent pour les touches
//!   multimédia et leurs propres panneaux.
//!
//! Le système affiche aussi le titre, l'artiste, l'album et la pochette du
//! morceau en cours. Windows veut la pochette dans un fichier : on l'écrit
//! dans le dossier temporaire (l'icône du lecteur si le morceau
//! n'en a pas, sinon Windows garderait celle du morceau d'avant).
//!
//! Les touches arrivent sur un autre fil que celui de la fenêtre : on se
//! contente de les mettre de côté et de réveiller la fenêtre, qui les traite
//! à la prochaine image (comme le menu de l'icône près de l'horloge).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use eframe::egui;
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

use crate::raccourcis::Commande;
use crate::tags;

/// De combien les touches « avance / retour rapide » se déplacent dans le
/// morceau (comme Q et D).
const SAUT: f32 = 5.0;

/// Ce que le panneau média affiche d'un morceau.
#[derive(Debug)]
struct Infos {
    chemin: PathBuf,
    titre: String,
    artiste: String,
    album: String,
    /// Le fichier image de la pochette, dans le dossier temporaire.
    pochette: Option<PathBuf>,
}

/// Ce que le système affiche de la lecture.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EtatMedia {
    Arrete,
    Pause,
    Lecture,
}

/// Les contrôles média du système.
pub struct ControlesMedia {
    /// Créés à la première image (Windows veut la fenêtre). Rien si le
    /// système les a refusés : le lecteur marche alors comme avant.
    controles: Option<MediaControls>,
    /// Vrai une fois la création tentée, réussie ou non.
    essaye: bool,
    touches: Option<Receiver<MediaControlEvent>>,
    /// Le dernier état donné au système.
    etat: EtatMedia,
    /// Le morceau dont le système affiche (ou va afficher) les informations.
    morceau: Option<PathBuf>,
    /// Ses informations, préparées en arrière-plan (lire les tags et écrire
    /// la pochette prend un moment).
    infos: Option<Receiver<Infos>>,
}

impl ControlesMedia {
    pub fn new() -> Self {
        Self {
            controles: None,
            essaye: false,
            touches: None,
            etat: EtatMedia::Arrete,
            morceau: None,
            infos: None,
        }
    }

    /// À appeler à chaque image : rend les commandes reçues des touches
    /// multimédia depuis la dernière image.
    pub fn surveiller(&mut self, ctx: &egui::Context, frame: &eframe::Frame) -> Vec<Commande> {
        if !self.essaye {
            self.essaye = true;
            self.creer(ctx, frame);
        }
        let Some(touches) = &self.touches else {
            return Vec::new();
        };
        let mut commandes = Vec::new();
        while let Ok(touche) = touches.try_recv() {
            commandes.extend(commande(&touche, self.etat));
        }
        commandes
    }

    /// Dit au système si on joue, si on est en pause ou arrêté. Windows
    /// s'en sert pour choisir entre « lecture » et « pause » quand on appuie
    /// sur la touche ⏯.
    pub fn etat(&mut self, etat: EtatMedia) {
        if etat == self.etat {
            return;
        }
        self.etat = etat;
        if let Some(controles) = &mut self.controles {
            let lecture = match etat {
                EtatMedia::Arrete => MediaPlayback::Stopped,
                EtatMedia::Pause => MediaPlayback::Paused { progress: None },
                EtatMedia::Lecture => MediaPlayback::Playing { progress: None },
            };
            let _ = controles.set_playback(lecture);
        }
    }

    /// À appeler à chaque image avec le morceau en cours : quand il change,
    /// ses informations sont préparées puis données au système. À l'arrêt
    /// (rien en cours), le panneau garde le dernier morceau.
    pub fn morceau(&mut self, chemin: Option<&Path>) {
        if let Some(reception) = &self.infos {
            if let Ok(infos) = reception.try_recv() {
                self.infos = None;
                // Un morceau plus récent a pu être lancé entre-temps.
                if self.morceau.as_deref() == Some(infos.chemin.as_path()) {
                    self.metadonnees(&infos);
                }
            }
        }
        let Some(chemin) = chemin else {
            return;
        };
        if self.controles.is_none() || self.morceau.as_deref() == Some(chemin) {
            return;
        }
        self.morceau = Some(chemin.to_path_buf());
        let (envoi, reception) = mpsc::channel();
        let chemin = chemin.to_path_buf();
        thread::spawn(move || {
            let _ = envoi.send(preparer(&chemin, &dossier_des_pochettes()));
        });
        self.infos = Some(reception);
    }

    fn metadonnees(&mut self, infos: &Infos) {
        let Some(controles) = &mut self.controles else {
            return;
        };
        let pochette = infos.pochette.as_deref().map(adresse_de_fichier);
        // Tout est donné, même vide : Windows garde sinon l'artiste ou
        // l'album du morceau d'avant.
        let _ = controles.set_metadata(MediaMetadata {
            title: Some(&infos.titre),
            artist: Some(&infos.artiste),
            album: Some(&infos.album),
            cover_url: pochette.as_deref(),
            duration: None,
        });
    }

    fn creer(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // Pendant les tests, plusieurs lecteurs tournent en même temps : on
        // ne s'inscrit pas auprès du système.
        if cfg!(test) {
            return;
        }
        let config = PlatformConfig {
            display_name: "Doll//Wave",
            dbus_name: "dollwave",
            hwnd: fenetre(frame),
        };
        // Windows exige la fenêtre : sans elle, souvlaki s'arrêterait net.
        if cfg!(windows) && config.hwnd.is_none() {
            return;
        }
        let Ok(mut controles) = MediaControls::new(config) else {
            return;
        };
        let (envoi, touches) = mpsc::channel();
        let ctx = ctx.clone();
        let branche = controles.attach(move |touche| {
            let _ = envoi.send(touche);
            ctx.request_repaint();
        });
        if branche.is_err() {
            return;
        }
        let _ = controles.set_playback(MediaPlayback::Stopped);
        self.controles = Some(controles);
        self.touches = Some(touches);
    }
}

/// La commande du lecteur qui correspond à une touche. « Lecture » et
/// « pause » ne font rien si le lecteur est déjà dans cet état.
fn commande(touche: &MediaControlEvent, etat: EtatMedia) -> Option<Commande> {
    use souvlaki::SeekDirection::{Backward, Forward};
    match touche {
        MediaControlEvent::Toggle => Some(Commande::LecturePause),
        MediaControlEvent::Play if etat != EtatMedia::Lecture => Some(Commande::LecturePause),
        MediaControlEvent::Pause if etat == EtatMedia::Lecture => Some(Commande::LecturePause),
        MediaControlEvent::Next => Some(Commande::Suivant),
        MediaControlEvent::Previous => Some(Commande::Precedent),
        MediaControlEvent::Stop => Some(Commande::Stop),
        MediaControlEvent::Seek(Forward) => Some(Commande::Deplacer(SAUT)),
        MediaControlEvent::Seek(Backward) => Some(Commande::Deplacer(-SAUT)),
        _ => None,
    }
}

/// Lit les tags du morceau et écrit sa pochette dans `dossier`.
fn preparer(chemin: &Path, dossier: &Path) -> Infos {
    let tags = tags::lire(chemin);
    Infos {
        chemin: chemin.to_path_buf(),
        titre: tags.titre.unwrap_or_else(|| sans_extension(chemin)),
        artiste: tags.artiste.unwrap_or_default(),
        album: tags.album.unwrap_or_default(),
        pochette: ecrire_pochette(chemin, dossier),
    }
}

/// Le nom du fichier sans « .mp3 », comme titre d'un morceau sans tags.
fn sans_extension(chemin: &Path) -> String {
    chemin
        .file_stem()
        .map(|nom| nom.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Le dossier temporaire où l'on range la pochette donnée au système.
fn dossier_des_pochettes() -> PathBuf {
    std::env::temp_dir().join("dollwave_pochettes")
}

/// Écrit la pochette du morceau (ou, s'il n'en a pas, l'icône du
/// lecteur) dans `dossier`, après avoir effacé les précédentes. Chaque
/// pochette a un nouveau nom : on n'écrit jamais dans un fichier que le
/// système est peut-être en train de lire.
fn ecrire_pochette(chemin: &Path, dossier: &Path) -> Option<PathBuf> {
    static NUMERO: AtomicU32 = AtomicU32::new(0);
    let (donnees, extension) = match tags::lire_pochette_brute(chemin).and_then(avec_extension) {
        Some(pochette) => pochette,
        None => (icone_png()?, "png"),
    };
    if let Ok(anciennes) = fs::read_dir(dossier) {
        for ancienne in anciennes.flatten() {
            let _ = fs::remove_file(ancienne.path());
        }
    }
    fs::create_dir_all(dossier).ok()?;
    let numero = NUMERO.fetch_add(1, Ordering::Relaxed);
    let fichier = dossier.join(format!("pochette-{numero}.{extension}"));
    fs::write(&fichier, donnees).ok()?;
    Some(fichier)
}

/// L'image avec l'extension de son format, si c'est un format que les
/// panneaux média savent afficher.
fn avec_extension(donnees: Vec<u8>) -> Option<(Vec<u8>, &'static str)> {
    let extension = match image::guess_format(&donnees).ok()? {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        _ => return None,
    };
    Some((donnees, extension))
}

/// L'icône du lecteur, en PNG.
fn icone_png() -> Option<Vec<u8>> {
    Some(super::ICONE_GRANDE.to_vec())
}

/// Le chemin d'un fichier écrit comme une adresse « file:// », la forme
/// qu'attend souvlaki pour une pochette. Sous Windows, souvlaki retire
/// juste « file:// » et ouvre ce qui reste : il faut le chemin tel quel.
#[cfg(windows)]
fn adresse_de_fichier(chemin: &Path) -> String {
    format!("file://{}", chemin.display())
}

/// Sous Linux, une vraie adresse : les caractères spéciaux (espaces,
/// accents…) sont écrits en « %xx ».
#[cfg(not(windows))]
fn adresse_de_fichier(chemin: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut adresse = String::from("file://");
    for &octet in chemin.as_os_str().as_bytes() {
        if octet.is_ascii_alphanumeric() || b"/-_.~".contains(&octet) {
            adresse.push(octet as char);
        } else {
            adresse.push_str(&format!("%{octet:02X}"));
        }
    }
    adresse
}

#[cfg(windows)]
fn fenetre(frame: &eframe::Frame) -> Option<*mut std::ffi::c_void> {
    super::windows::fenetre(frame)
}

#[cfg(not(windows))]
fn fenetre(_frame: &eframe::Frame) -> Option<*mut std::ffi::c_void> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn la_touche_lecture_pause_bascule() {
        for etat in [EtatMedia::Arrete, EtatMedia::Pause, EtatMedia::Lecture] {
            assert_eq!(
                commande(&MediaControlEvent::Toggle, etat),
                Some(Commande::LecturePause)
            );
        }
    }

    #[test]
    fn lecture_et_pause_ne_font_rien_si_deja_dans_cet_etat() {
        let lecture = MediaControlEvent::Play;
        let pause = MediaControlEvent::Pause;
        assert_eq!(commande(&lecture, EtatMedia::Lecture), None);
        assert_eq!(
            commande(&lecture, EtatMedia::Pause),
            Some(Commande::LecturePause)
        );
        assert_eq!(
            commande(&lecture, EtatMedia::Arrete),
            Some(Commande::LecturePause)
        );
        assert_eq!(commande(&pause, EtatMedia::Pause), None);
        assert_eq!(commande(&pause, EtatMedia::Arrete), None);
        assert_eq!(
            commande(&pause, EtatMedia::Lecture),
            Some(Commande::LecturePause)
        );
    }

    #[test]
    fn suivant_precedent_stop_et_saut() {
        use souvlaki::SeekDirection::{Backward, Forward};
        let etat = EtatMedia::Lecture;
        assert_eq!(
            commande(&MediaControlEvent::Next, etat),
            Some(Commande::Suivant)
        );
        assert_eq!(
            commande(&MediaControlEvent::Previous, etat),
            Some(Commande::Precedent)
        );
        assert_eq!(
            commande(&MediaControlEvent::Stop, etat),
            Some(Commande::Stop)
        );
        assert_eq!(
            commande(&MediaControlEvent::Seek(Forward), etat),
            Some(Commande::Deplacer(SAUT))
        );
        assert_eq!(
            commande(&MediaControlEvent::Seek(Backward), etat),
            Some(Commande::Deplacer(-SAUT))
        );
        assert_eq!(commande(&MediaControlEvent::Raise, etat), None);
    }

    #[cfg(windows)]
    #[test]
    fn adresse_de_pochette() {
        assert_eq!(
            adresse_de_fichier(Path::new(
                r"C:\Users\Luce\AppData\Local\Temp\pochette-0.jpg"
            )),
            r"file://C:\Users\Luce\AppData\Local\Temp\pochette-0.jpg"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn adresse_de_pochette() {
        assert_eq!(
            adresse_de_fichier(Path::new("/tmp/Mon album/pochette-0.jpg")),
            "file:///tmp/Mon%20album/pochette-0.jpg"
        );
        assert_eq!(
            adresse_de_fichier(Path::new("/tmp/Élodie.png")),
            "file:///tmp/%C3%89lodie.png"
        );
    }

    /// Un dossier vide pour un test.
    fn dossier_de_test(nom: &str) -> PathBuf {
        let dossier = std::env::temp_dir().join(format!("dollwave_test_media_{nom}"));
        let _ = fs::remove_dir_all(&dossier);
        fs::create_dir_all(&dossier).unwrap();
        dossier
    }

    #[test]
    fn sans_tags_le_titre_est_le_nom_du_fichier_et_la_pochette_l_icone() {
        let dossier = dossier_de_test("sans_tags");
        let mp3 = dossier.join("Mon morceau.mp3");
        fs::copy("tests/glissando.mp3", &mp3).unwrap();
        let pochettes = dossier.join("pochettes");
        let infos = preparer(&mp3, &pochettes);
        assert_eq!(infos.titre, "Mon morceau");
        assert_eq!(infos.artiste, "");
        assert_eq!(infos.album, "");
        let pochette = infos.pochette.unwrap();
        assert_eq!(pochette.extension().unwrap(), "png");
        let image = image::open(&pochette).unwrap();
        // L'icône du lecteur, en 256 pixels.
        assert_eq!(image.width(), 256);
    }

    #[test]
    fn titre_artiste_album_et_pochette_viennent_des_tags() {
        use id3::TagLike;
        let dossier = dossier_de_test("avec_tags");
        let mp3 = dossier.join("morceau.mp3");
        fs::copy("tests/glissando.mp3", &mp3).unwrap();
        let jpeg = {
            let image = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 0, 100]));
            let mut jpeg = Cursor::new(Vec::new());
            image.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
            jpeg.into_inner()
        };
        let mut tag = id3::Tag::new();
        tag.set_title("Néon");
        tag.set_artist("Luce");
        tag.set_album("Nuit");
        tag.add_frame(id3::frame::Picture {
            mime_type: "image/jpeg".into(),
            picture_type: id3::frame::PictureType::CoverFront,
            description: String::new(),
            data: jpeg.clone(),
        });
        tag.write_to_path(&mp3, id3::Version::Id3v24).unwrap();

        let pochettes = dossier.join("pochettes");
        let infos = preparer(&mp3, &pochettes);
        assert_eq!(infos.titre, "Néon");
        assert_eq!(infos.artiste, "Luce");
        assert_eq!(infos.album, "Nuit");
        let pochette = infos.pochette.unwrap();
        assert_eq!(pochette.extension().unwrap(), "jpg");
        assert_eq!(fs::read(&pochette).unwrap(), jpeg);
    }

    #[test]
    fn une_nouvelle_pochette_efface_l_ancienne() {
        let dossier = dossier_de_test("efface");
        let mp3 = dossier.join("a.mp3");
        fs::copy("tests/glissando.mp3", &mp3).unwrap();
        let pochettes = dossier.join("pochettes");
        let premiere = ecrire_pochette(&mp3, &pochettes).unwrap();
        let seconde = ecrire_pochette(&mp3, &pochettes).unwrap();
        assert_ne!(premiere, seconde);
        assert!(!premiere.exists());
        assert_eq!(fs::read_dir(&pochettes).unwrap().count(), 1);
    }
}
