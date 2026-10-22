//! Les écoutes : combien de fois on a écouté chaque morceau, et quand, pour
//! remplir tout seul la playlist « Les plus écoutés ».
//!
//! Un morceau compte une écoute quand on en a écouté au moins la moitié (ou
//! 4 minutes pour un très long morceau) : un morceau passé au bout de
//! quelques secondes ne compte pas. Avancer dans le morceau (clic sur l'onde)
//! ne compte pas non plus : seul le temps réellement écouté est pris.
//!
//! Les écoutes récentes pèsent plus lourd : chaque écoute perd la moitié de
//! son poids tous les 30 jours. Un morceau écouté 4 fois cette semaine passe
//! ainsi devant un morceau écouté 10 fois il y a six mois.
//!
//! Le tout est mémorisé dans le dossier de réglages (ecoutes.json).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::bibliotheque::memoriser;
use crate::systeme::fichier_de_reglage;

/// Le fichier où l'on mémorise les écoutes.
const REGLAGE_ECOUTES: &str = "ecoutes.json";
/// Au bout de combien de temps une écoute ne pèse plus que la moitié.
const DEMI_VIE: f64 = 30.0 * 24.0 * 3600.0;
/// Le temps d'écoute au-delà duquel un long morceau compte toujours.
const ECOUTE_MAX: Duration = Duration::from_secs(4 * 60);
/// Combien de morceaux dans « Les plus écoutés ».
pub const TAILLE_DE_LA_PLAYLIST: usize = 50;

/// Ce qu'on sait des écoutes d'un morceau.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fiche {
    /// Les écoutes, pondérées, telles qu'elles étaient à la dernière écoute.
    poids: f64,
    /// La date de la dernière écoute, en secondes depuis 1970.
    derniere: u64,
}

impl Fiche {
    /// Le poids des écoutes à la date `maintenant` : il a fondu depuis la
    /// dernière écoute (de moitié tous les 30 jours).
    fn poids_a(&self, maintenant: u64) -> f64 {
        let ecoule = maintenant.saturating_sub(self.derniere) as f64;
        self.poids * 0.5_f64.powf(ecoule / DEMI_VIE)
    }
}

/// Les écoutes de tous les morceaux.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ecoutes {
    fiches: HashMap<PathBuf, Fiche>,
}

/// La date d'aujourd'hui, en secondes depuis 1970.
pub fn maintenant() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Le temps qu'il faut écouter un morceau de cette durée pour qu'il compte.
pub fn seuil(duree: Duration) -> Duration {
    (duree / 2).min(ECOUTE_MAX)
}

impl Ecoutes {
    /// Les écoutes mémorisées (aucune s'il n'y en a pas encore, ou si le
    /// fichier est abîmé).
    pub fn charger() -> Self {
        fichier_de_reglage(REGLAGE_ECOUTES)
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|texte| Self::depuis_json(&texte))
            .unwrap_or_default()
    }

    /// Mémorise les écoutes pour les prochains lancements.
    pub fn memoriser(&self) {
        memoriser(REGLAGE_ECOUTES, &self.vers_json());
    }

    /// Compte une écoute de `morceau`, à la date `maintenant`.
    pub fn compter(&mut self, morceau: &Path, maintenant: u64) {
        let poids = self
            .fiches
            .get(morceau)
            .map_or(0.0, |f| f.poids_a(maintenant));
        self.fiches.insert(
            morceau.to_path_buf(),
            Fiche {
                poids: poids + 1.0,
                derniere: maintenant,
            },
        );
    }

    /// Les morceaux les plus écoutés à la date `maintenant` (les écoutes
    /// récentes pesant plus lourd), au plus `combien`, du premier au
    /// dernier. Ceux dont le fichier a disparu (`existe`) sont écartés.
    pub fn plus_ecoutes(
        &self,
        maintenant: u64,
        combien: usize,
        existe: impl Fn(&Path) -> bool,
    ) -> Vec<PathBuf> {
        let mut classement: Vec<(&PathBuf, f64)> = self
            .fiches
            .iter()
            .map(|(morceau, fiche)| (morceau, fiche.poids_a(maintenant)))
            .collect();
        // Du plus lourd au plus léger ; à égalité, par nom (toujours le même ordre).
        classement.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        classement
            .into_iter()
            .map(|(morceau, _)| morceau)
            .filter(|morceau| existe(morceau))
            .take(combien)
            .cloned()
            .collect()
    }

    fn vers_json(&self) -> String {
        let fiches: Vec<Value> = self
            .fiches
            .iter()
            .map(|(morceau, fiche)| {
                json!({
                    "morceau": morceau.to_string_lossy(),
                    "poids": fiche.poids,
                    "derniere": fiche.derniere,
                })
            })
            .collect();
        serde_json::to_string_pretty(&json!({ "ecoutes": fiches })).unwrap_or_default()
    }

    fn depuis_json(texte: &str) -> Option<Self> {
        let valeur: Value = serde_json::from_str(texte).ok()?;
        let fiches = valeur["ecoutes"]
            .as_array()?
            .iter()
            .filter_map(|fiche| {
                let morceau = PathBuf::from(fiche["morceau"].as_str()?);
                let poids = fiche["poids"]
                    .as_f64()
                    .filter(|p| p.is_finite() && *p > 0.0)?;
                let derniere = fiche["derniere"].as_u64()?;
                Some((morceau, Fiche { poids, derniere }))
            })
            .collect();
        Some(Self { fiches })
    }
}

/// Suit l'écoute du morceau en cours, pour savoir quand il compte.
#[derive(Default)]
pub struct Suivi {
    /// Le morceau suivi (son adresse en mémoire : un morceau relancé est
    /// rouvert, il a donc une nouvelle adresse et recommence à zéro).
    morceau: usize,
    /// Où on en était dans le morceau au dernier coup d'œil.
    position: Duration,
    /// Le temps réellement écouté depuis qu'il a commencé.
    ecoute: Duration,
    /// Il a déjà été compté (une seule fois par lecture).
    compte: bool,
}

impl Suivi {
    /// Un coup d'œil sur la lecture : `morceau` est l'adresse du morceau en
    /// cours, `position` où on en est, `duree` sa durée et `ecoule` le temps
    /// réel passé depuis le dernier coup d'œil. Vrai quand le morceau vient
    /// d'être assez écouté pour compter.
    pub fn avancer(
        &mut self,
        morceau: usize,
        position: Duration,
        duree: Duration,
        ecoule: Duration,
    ) -> bool {
        if morceau != self.morceau {
            *self = Self {
                morceau,
                position,
                ..Self::default()
            };
        }
        // La position a avancé : on l'a écoutée, sauf si elle a sauté plus
        // loin que le temps passé (on a cliqué plus loin dans le morceau).
        if position > self.position {
            let avance = position - self.position;
            if avance <= ecoule + Duration::from_millis(500) {
                self.ecoute += avance;
            }
        }
        self.position = position;
        if !self.compte && !duree.is_zero() && self.ecoute >= seuil(duree) {
            self.compte = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOUR: u64 = 24 * 3600;

    fn existe(_: &Path) -> bool {
        true
    }

    #[test]
    fn un_morceau_ecoute_recemment_passe_devant_un_vieux_favori() {
        let mut ecoutes = Ecoutes::default();
        let debut = 1_700_000_000;
        // Écouté 10 fois il y a six mois…
        for i in 0..10 {
            ecoutes.compter(Path::new("vieux.mp3"), debut + i * 60);
        }
        // … et 4 fois cette semaine.
        let aujourdhui = debut + 180 * JOUR;
        for i in 0..4 {
            ecoutes.compter(Path::new("nouveau.mp3"), aujourdhui - i * JOUR);
        }
        assert_eq!(
            ecoutes.plus_ecoutes(aujourdhui, 50, existe),
            vec![PathBuf::from("nouveau.mp3"), PathBuf::from("vieux.mp3")]
        );
        // Le même jour, c'est le nombre d'écoutes qui départage.
        assert_eq!(
            ecoutes.plus_ecoutes(debut + 3600, 50, existe)[0],
            PathBuf::from("vieux.mp3")
        );
    }

    #[test]
    fn une_ecoute_perd_la_moitie_de_son_poids_en_30_jours() {
        let fiche = Fiche {
            poids: 4.0,
            derniere: 1000,
        };
        assert_eq!(fiche.poids_a(1000), 4.0);
        assert!((fiche.poids_a(1000 + 30 * JOUR) - 2.0).abs() < 1e-9);
        assert!((fiche.poids_a(1000 + 60 * JOUR) - 1.0).abs() < 1e-9);
        // Une horloge qui recule ne fait pas grossir le poids.
        assert_eq!(fiche.poids_a(0), 4.0);
    }

    #[test]
    fn la_playlist_est_limitee_et_sans_les_fichiers_disparus() {
        let mut ecoutes = Ecoutes::default();
        for i in 0..60 {
            ecoutes.compter(&PathBuf::from(format!("{i:02}.mp3")), 1000);
        }
        let liste = ecoutes.plus_ecoutes(1000, TAILLE_DE_LA_PLAYLIST, |m| m != Path::new("00.mp3"));
        assert_eq!(liste.len(), TAILLE_DE_LA_PLAYLIST);
        assert!(!liste.contains(&PathBuf::from("00.mp3")));
    }

    #[test]
    fn les_ecoutes_se_relisent_telles_qu_on_les_a_ecrites() {
        let mut ecoutes = Ecoutes::default();
        ecoutes.compter(Path::new("C:\\Musique\\Été.mp3"), 1_700_000_000);
        ecoutes.compter(Path::new("C:\\Musique\\Été.mp3"), 1_700_000_000);
        ecoutes.compter(Path::new("/home/luce/2.mp3"), 1_700_000_000);
        assert_eq!(Ecoutes::depuis_json(&ecoutes.vers_json()), Some(ecoutes));
        assert_eq!(Ecoutes::depuis_json(""), None);
        // Une fiche abîmée est écartée, les autres sont gardées.
        let abime = r#"{"ecoutes":[{"morceau":"a.mp3","poids":"x","derniere":1},
            {"morceau":"b.mp3","poids":1.0,"derniere":5}]}"#;
        let relu = Ecoutes::depuis_json(abime).unwrap();
        assert_eq!(
            relu.plus_ecoutes(5, 50, existe),
            vec![PathBuf::from("b.mp3")]
        );
    }

    fn s(secondes: u64) -> Duration {
        Duration::from_secs(secondes)
    }

    #[test]
    fn un_morceau_compte_a_la_moitie_et_une_seule_fois() {
        let mut suivi = Suivi::default();
        let duree = s(200);
        let mut comptes = 0;
        for t in 0..=200 {
            if suivi.avancer(1, s(t), duree, s(1)) {
                comptes += 1;
                assert_eq!(t, 100);
            }
        }
        assert_eq!(comptes, 1);
        // Relancé (nouvelle adresse) : il peut compter à nouveau.
        let relance = (0..=200)
            .filter(|&t| suivi.avancer(2, s(t), duree, s(1)))
            .count();
        assert_eq!(relance, 1);
    }

    #[test]
    fn un_long_mix_compte_au_bout_de_4_minutes() {
        let mut suivi = Suivi::default();
        let compte = (0..=3600).find(|&t| suivi.avancer(1, s(t), s(3600), s(1)));
        assert_eq!(compte, Some(240));
    }

    #[test]
    fn sauter_plus_loin_dans_le_morceau_ne_compte_pas() {
        let mut suivi = Suivi::default();
        let duree = s(200);
        assert!(!suivi.avancer(1, s(0), duree, s(0)));
        assert!(!suivi.avancer(1, s(10), duree, s(10)));
        // Un clic sur l'onde à 190 s : ce saut n'est pas de l'écoute.
        assert!(!suivi.avancer(1, s(190), duree, Duration::from_millis(16)));
        assert!(!suivi.avancer(1, s(200), duree, s(10)));
        // Revenir en arrière ne retire rien, et la réécoute compte.
        let compte = (20..=200).find(|&t| suivi.avancer(1, s(t), duree, s(1)));
        assert_eq!(compte, Some(100));
    }

    #[test]
    fn en_pause_rien_n_avance() {
        let mut suivi = Suivi::default();
        assert!(!suivi.avancer(1, s(30), s(60), s(0)));
        for _ in 0..100 {
            assert!(!suivi.avancer(1, s(30), s(60), s(5)));
        }
    }
}
