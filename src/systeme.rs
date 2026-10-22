//! Tout ce qui dépend du système d'exploitation (Windows ou Linux) est
//! regroupé ici, pour que le reste du lecteur n'ait pas à s'en soucier.
//! Les dossiers sont trouvés par la bibliothèque `dirs`, qui connaît les
//! usages de chaque système. Le code propre à un seul système est dans un
//! sous-module (systeme/windows.rs).

use std::path::{Path, PathBuf};

mod instance;
mod media;
#[cfg(windows)]
mod windows;

pub use instance::{demarrer, Boite, Demarrage};
pub use media::{ControlesMedia, EtatMedia};
#[cfg(windows)]
pub use windows::{autoriser_le_premier_plan, ZoneDeNotification};

/// Le nom de notre dossier de réglages.
const NOTRE_DOSSIER: &str = "lecteur_mp3";

/// Le dossier Musique de l'utilisateur, celui que le système connaît :
/// - Windows : C:\Users\<nom>\Music (ou là où il a été déplacé, OneDrive…)
/// - Linux : ~/Musique (ou ~/Music selon la langue)
///
/// Si le système n'en connaît pas, on prend « Music » dans le dossier
/// personnel.
pub fn dossier_musique() -> PathBuf {
    dirs::audio_dir()
        .or_else(|| dirs::home_dir().map(|maison| maison.join("Music")))
        .unwrap_or_default()
}

/// Un petit fichier de réglage, par exemple :
/// - Windows : %APPDATA%\lecteur_mp3\dossier_musique.txt
/// - Linux : ~/.config/lecteur_mp3/dossier_musique.txt
pub fn fichier_de_reglage(nom: &str) -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(NOTRE_DOSSIER).join(nom))
}

/// Ouvre le dossier d'un fichier dans le gestionnaire de fichiers, avec ce
/// fichier sélectionné (sous Windows : l'Explorateur). Sous Linux, le
/// dossier s'ouvre simplement, sans sélection.
pub fn montrer_le_fichier(chemin: &Path) -> Result<(), String> {
    let erreur = |e: std::io::Error| format!("Impossible d'ouvrir le dossier : {e}");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // L'Explorateur attend exactement /select,"C:\chemin\du fichier.mp3" :
        // on écrit les guillemets nous-mêmes. Il répond « échec » même quand
        // tout va bien : on ne regarde que s'il a pu être lancé.
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", chemin.display()))
            .spawn()
            .map(drop)
            .map_err(erreur)
    }
    #[cfg(not(windows))]
    {
        let dossier = chemin.parent().unwrap_or(Path::new("."));
        std::process::Command::new("xdg-open")
            .arg(dossier)
            .spawn()
            .map(drop)
            .map_err(erreur)
    }
}

/// Ailleurs que sous Windows, pas encore d'icône près de l'horloge : la
/// croix ferme le lecteur, comme d'habitude. (Sous Linux, il faudra une autre
/// bibliothèque que sous Windows, voir la to do.)
#[cfg(not(windows))]
pub struct ZoneDeNotification;

#[cfg(not(windows))]
impl ZoneDeNotification {
    pub fn new(_ctx: &eframe::egui::Context) -> Self {
        Self
    }

    pub fn surveiller(
        &mut self,
        _ctx: &eframe::egui::Context,
        _frame: &eframe::Frame,
    ) -> Vec<crate::raccourcis::Commande> {
        Vec::new()
    }

    pub fn infobulle(&mut self, _morceau: Option<&str>) {}

    /// Ferme le lecteur.
    pub fn quitter(&mut self, ctx: &eframe::egui::Context) {
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
    }

    /// Ramène la fenêtre au premier plan (même réduite).
    pub fn afficher(&mut self, ctx: &eframe::egui::Context) {
        use eframe::egui::ViewportCommand;
        if ctx.input(|i| i.viewport().minimized == Some(true)) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        }
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }
}

/// Rien à faire hors de Windows (voir la version Windows).
#[cfg(not(windows))]
pub fn autoriser_le_premier_plan() {}

/// L'icône du lecteur (plot néon sur un sol quadrillé synthwave), en grand pour
/// la fenêtre et la barre des tâches. Le dessin d'origine est
/// assets/icone/icone.svg ; le .exe reçoit assets/icone/icone.ico (voir
/// build.rs).
const ICONE_GRANDE: &[u8] = include_bytes!("../assets/icone/icone-256.png");

/// La même icône en 32 pixels, pour la zone près de l'horloge.
#[cfg_attr(not(windows), allow(dead_code))]
const ICONE_PETITE: &[u8] = include_bytes!("../assets/icone/icone-32.png");

/// L'icône de la fenêtre du lecteur.
pub fn icone_fenetre() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(ICONE_GRANDE).unwrap_or_default()
}

/// L'icône près de l'horloge : ses pixels (rouge, vert, bleu, opacité),
/// ligne par ligne, et sa taille.
#[cfg_attr(not(windows), allow(dead_code))]
fn icone_petite() -> Option<(Vec<u8>, u32, u32)> {
    let image = image::load_from_memory(ICONE_PETITE).ok()?.into_rgba8();
    let (largeur, hauteur) = image.dimensions();
    Some((image.into_raw(), largeur, hauteur))
}

/// Windows coupe l'infobulle de l'icône au-delà de 127 caractères.
const INFOBULLE_MAX: usize = 120;

/// Le texte affiché quand la souris passe sur l'icône près de l'horloge.
#[cfg_attr(not(windows), allow(dead_code))]
fn texte_d_infobulle(morceau: Option<&str>) -> String {
    let texte = match morceau {
        Some(morceau) => format!("Lecteur MP3 · {morceau}"),
        None => "Lecteur MP3".to_string(),
    };
    if texte.chars().count() <= INFOBULLE_MAX {
        return texte;
    }
    let mut coupe: String = texte.chars().take(INFOBULLE_MAX - 1).collect();
    coupe.push('…');
    coupe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_reglages_vont_dans_notre_dossier() {
        let fichier = fichier_de_reglage("favoris.txt").expect("dossier de réglages");
        assert!(fichier.ends_with("lecteur_mp3/favoris.txt"));
        assert!(fichier.is_absolute());
        if cfg!(windows) {
            // Même dossier qu'avant : les réglages déjà enregistrés sont gardés.
            let appdata = PathBuf::from(std::env::var_os("APPDATA").unwrap());
            assert_eq!(fichier, appdata.join("lecteur_mp3").join("favoris.txt"));
        }
    }

    #[test]
    fn il_y_a_toujours_un_dossier_musique() {
        assert!(dossier_musique().is_absolute());
    }

    #[test]
    fn les_icones_se_lisent() {
        let fenetre = icone_fenetre();
        assert_eq!((fenetre.width, fenetre.height), (256, 256));
        let (pixels, largeur, hauteur) = icone_petite().expect("icône 32 px");
        assert_eq!((largeur, hauteur), (32, 32));
        assert_eq!(pixels.len(), 32 * 32 * 4);
        assert_eq!(pixels[3], 0, "coin arrondi transparent");
    }

    #[test]
    fn infobulle_avec_ou_sans_morceau() {
        assert_eq!(texte_d_infobulle(None), "Lecteur MP3");
        assert_eq!(
            texte_d_infobulle(Some("Zebda – Tomber la chemise")),
            "Lecteur MP3 · Zebda – Tomber la chemise"
        );
    }

    #[test]
    fn infobulle_trop_longue_coupee() {
        let long = "é".repeat(300);
        let texte = texte_d_infobulle(Some(&long));
        assert_eq!(texte.chars().count(), INFOBULLE_MAX);
        assert!(texte.ends_with('…'));
    }
}
