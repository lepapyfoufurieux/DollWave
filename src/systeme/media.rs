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
//! Les touches arrivent sur un autre fil que celui de la fenêtre : on se
//! contente de les mettre de côté et de réveiller la fenêtre, qui les traite
//! à la prochaine image (comme le menu de l'icône près de l'horloge).

use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use eframe::egui;
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

use crate::raccourcis::Commande;

/// De combien les touches « avance / retour rapide » se déplacent dans le
/// morceau (comme Q et D).
const SAUT: f32 = 5.0;

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
}

impl ControlesMedia {
    pub fn new() -> Self {
        Self {
            controles: None,
            essaye: false,
            touches: None,
            etat: EtatMedia::Arrete,
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

    /// Les informations du morceau en cours, pour les panneaux média du
    /// système. (Pas encore utilisé : ce sera le panneau média de Windows.)
    #[allow(dead_code)]
    pub fn metadonnees(
        &mut self,
        titre: Option<&str>,
        artiste: Option<&str>,
        album: Option<&str>,
        pochette: Option<&Path>,
        duree: Option<Duration>,
    ) {
        let Some(controles) = &mut self.controles else {
            return;
        };
        let pochette = pochette.map(adresse_de_fichier);
        let _ = controles.set_metadata(MediaMetadata {
            title: titre,
            artist: artiste,
            album,
            cover_url: pochette.as_deref(),
            duration: duree,
        });
    }

    fn creer(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // Pendant les tests, plusieurs lecteurs tournent en même temps : on
        // ne s'inscrit pas auprès du système.
        if cfg!(test) {
            return;
        }
        let config = PlatformConfig {
            display_name: "Lecteur MP3",
            dbus_name: "lecteur_mp3",
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

/// Le chemin d'un fichier écrit comme une adresse « file:/// », la forme
/// que les systèmes attendent pour une pochette.
fn adresse_de_fichier(chemin: &Path) -> String {
    let texte = chemin.to_string_lossy().replace('\\', "/");
    let texte = texte.trim_start_matches('/');
    format!("file:///{}", texte.replace(' ', "%20"))
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

    #[test]
    fn adresse_de_pochette() {
        assert_eq!(
            adresse_de_fichier(Path::new(r"C:\Musique\Mon album\cover.jpg")),
            "file:///C:/Musique/Mon%20album/cover.jpg"
        );
        assert_eq!(
            adresse_de_fichier(Path::new("/home/luce/pochette.png")),
            "file:///home/luce/pochette.png"
        );
    }
}
