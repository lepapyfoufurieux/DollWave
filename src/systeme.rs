//! Tout ce qui dépend du système d'exploitation (Windows ou Linux) est
//! regroupé ici, pour que le reste du lecteur n'ait pas à s'en soucier.
//! Les dossiers sont trouvés par la bibliothèque `dirs`, qui connaît les
//! usages de chaque système.

use std::path::PathBuf;

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
}
