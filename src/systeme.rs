//! Tout ce qui dépend du système d'exploitation (Windows ou Linux) est
//! regroupé ici, pour que le reste du lecteur n'ait pas à s'en soucier.
//! Les dossiers sont trouvés par la bibliothèque `dirs`, qui connaît les
//! usages de chaque système. Le code propre à un seul système est dans un
//! sous-module (systeme/windows.rs).

use std::path::PathBuf;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::ZoneDeNotification;

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

/// Ailleurs que sous Windows, pas encore d'icône près de l'horloge : la
/// fenêtre se réduit normalement. (Sous Linux, il faudra une autre
/// bibliothèque que sous Windows, voir la to do.)
#[cfg(not(windows))]
pub struct ZoneDeNotification;

#[cfg(not(windows))]
impl ZoneDeNotification {
    pub fn new() -> Self {
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
}

/// Taille de l'icône près de l'horloge, en pixels.
#[cfg_attr(not(windows), allow(dead_code))]
const TAILLE_ICONE: u32 = 32;

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

/// Dessine l'icône du lecteur : un petit disque vinyle avec son étiquette
/// orange. Rend les pixels (rouge, vert, bleu, opacité), ligne par ligne.
#[cfg_attr(not(windows), allow(dead_code))]
fn icone_vinyle(taille: u32) -> Vec<u8> {
    let centre = taille as f32 / 2.0;
    let mut pixels = Vec::with_capacity((taille * taille * 4) as usize);
    for y in 0..taille {
        for x in 0..taille {
            // Distance au centre, rapportée au rayon (1.0 = bord du disque).
            let dx = x as f32 + 0.5 - centre;
            let dy = y as f32 + 0.5 - centre;
            let r = (dx * dx + dy * dy).sqrt() / centre;
            let (couleur, opacite) = if !(0.08..=1.0).contains(&r) {
                // Hors du disque, ou le trou du milieu : transparent.
                ([0, 0, 0], 0)
            } else if r < 0.38 {
                ([240, 130, 40], 255)
            } else if ((r * 10.0).round() as u32).is_multiple_of(2) {
                // Les sillons, un peu plus clairs.
                ([60, 60, 60], 255)
            } else {
                ([25, 25, 25], 255)
            };
            pixels.extend_from_slice(&couleur);
            pixels.push(opacite);
        }
    }
    pixels
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
    fn l_icone_est_un_disque_troue() {
        let pixels = icone_vinyle(TAILLE_ICONE);
        assert_eq!(pixels.len(), (TAILLE_ICONE * TAILLE_ICONE * 4) as usize);
        let opacite = |x: u32, y: u32| pixels[((y * TAILLE_ICONE + x) * 4 + 3) as usize];
        let milieu = TAILLE_ICONE / 2;
        assert_eq!(opacite(0, 0), 0, "coin transparent");
        assert_eq!(opacite(milieu, milieu), 0, "trou du milieu");
        assert_eq!(opacite(milieu, 2), 255, "bord du disque");
        assert_eq!(opacite(milieu, milieu - 4), 255, "étiquette");
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
