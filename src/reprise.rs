//! Reprendre l'écoute au lancement : la liste de lecture, le morceau en
//! cours, la seconde où on l'avait quitté, et les modes aléatoire et
//! répétition. (Le volume, lui, n'est pas retenu : 50 % au départ.)
//!
//! Le tout est mémorisé dans le dossier de réglages (reprise.json), à la
//! fermeture et toutes les 10 secondes si quelque chose a changé, pour ne
//! rien perdre si le PC s'éteint d'un coup.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::bibliotheque::memoriser;
use crate::systeme::fichier_de_reglage;

/// Le fichier où l'on mémorise l'écoute en cours.
const REGLAGE_REPRISE: &str = "reprise.json";
/// Tous les combien on mémorise l'écoute (si elle a changé).
pub const INTERVALLE: Duration = Duration::from_secs(10);

/// L'écoute telle qu'on l'a laissée.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reprise {
    pub morceaux: Vec<PathBuf>,
    /// Le numéro du morceau en cours dans la liste, s'il y en a un.
    pub courant: Option<usize>,
    /// Où on en était dans ce morceau, en secondes.
    pub position: f32,
    pub aleatoire: bool,
    pub repetition: bool,
}

impl Reprise {
    /// L'écoute mémorisée au dernier lancement (rien s'il n'y en a pas, ou
    /// si le fichier est abîmé).
    pub fn charger() -> Option<Self> {
        let fichier = fichier_de_reglage(REGLAGE_REPRISE)?;
        Self::depuis_json(&std::fs::read_to_string(fichier).ok()?)
    }

    /// Mémorise l'écoute pour le prochain lancement.
    pub fn memoriser(&self) {
        memoriser(REGLAGE_REPRISE, &self.vers_json());
    }

    fn vers_json(&self) -> String {
        let morceaux: Vec<_> = self
            .morceaux
            .iter()
            .map(|m| m.to_string_lossy().into_owned())
            .collect();
        let texte = json!({
            "morceaux": morceaux,
            "courant": self.courant,
            "position": self.position,
            "aleatoire": self.aleatoire,
            "repetition": self.repetition,
        });
        serde_json::to_string_pretty(&texte).unwrap_or_default()
    }

    fn depuis_json(texte: &str) -> Option<Self> {
        let valeur: Value = serde_json::from_str(texte).ok()?;
        let morceaux: Vec<PathBuf> = valeur["morceaux"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect();
        // Un numéro de morceau en dehors de la liste ne compte pas.
        let courant = valeur["courant"]
            .as_u64()
            .map(|n| n as usize)
            .filter(|&n| n < morceaux.len());
        let position = valeur["position"].as_f64().unwrap_or(0.0) as f32;
        Some(Self {
            morceaux,
            courant,
            position: if position.is_finite() {
                position.max(0.0)
            } else {
                0.0
            },
            aleatoire: valeur["aleatoire"].as_bool().unwrap_or(false),
            repetition: valeur["repetition"].as_bool().unwrap_or(false),
        })
    }

    /// Retire les morceaux dont le fichier a disparu (`existe` dit s'il est
    /// toujours là). Si c'est le morceau en cours, on se place au début du
    /// suivant (celui qui a pris sa place), ou nulle part s'il n'y en a plus.
    pub fn sans_les_disparus(self, existe: impl Fn(&Path) -> bool) -> Self {
        let mut courant = None;
        let mut position = 0.0;
        let mut morceaux = Vec::new();
        for (n, morceau) in self.morceaux.into_iter().enumerate() {
            if !existe(&morceau) {
                continue;
            }
            match self.courant {
                // Le morceau en cours est toujours là : on garde sa position.
                Some(c) if c == n => {
                    courant = Some(morceaux.len());
                    position = self.position;
                }
                // Le morceau en cours a disparu : le premier présent après lui.
                Some(c) if c < n && courant.is_none() => courant = Some(morceaux.len()),
                _ => {}
            }
            morceaux.push(morceau);
        }
        Self {
            morceaux,
            courant,
            position,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn liste(noms: &[&str]) -> Vec<PathBuf> {
        noms.iter().map(PathBuf::from).collect()
    }

    fn exemple() -> Reprise {
        Reprise {
            morceaux: liste(&["C:\\Musique\\Été 1.mp3", "/home/luce/2.mp3", "3.mp3"]),
            courant: Some(1),
            position: 83.5,
            aleatoire: true,
            repetition: false,
        }
    }

    #[test]
    fn l_ecoute_se_relit_telle_qu_on_l_a_ecrite() {
        let reprise = exemple();
        assert_eq!(Reprise::depuis_json(&reprise.vers_json()), Some(reprise));
        let vide = Reprise::default();
        assert_eq!(Reprise::depuis_json(&vide.vers_json()), Some(vide));
    }

    #[test]
    fn un_fichier_abime_ne_donne_rien_ou_des_valeurs_sures() {
        assert_eq!(Reprise::depuis_json(""), None);
        assert_eq!(Reprise::depuis_json("{ pas du json"), None);
        assert_eq!(Reprise::depuis_json("{}"), None);
        // Morceau en cours hors de la liste, position négative : ignorés.
        let reprise =
            Reprise::depuis_json(r#"{"morceaux": ["a.mp3"], "courant": 5, "position": -3}"#)
                .unwrap();
        assert_eq!(reprise.morceaux, liste(&["a.mp3"]));
        assert_eq!(reprise.courant, None);
        assert_eq!(reprise.position, 0.0);
        assert!(!reprise.aleatoire && !reprise.repetition);
    }

    #[test]
    fn les_morceaux_disparus_sont_retires() {
        let reprise = Reprise {
            morceaux: liste(&["a", "b", "c", "d"]),
            courant: Some(2),
            position: 42.0,
            ..Reprise::default()
        };
        // « b » a disparu : le morceau en cours (« c ») reste à sa seconde.
        let reste = reprise.clone().sans_les_disparus(|m| m != Path::new("b"));
        assert_eq!(reste.morceaux, liste(&["a", "c", "d"]));
        assert_eq!(reste.courant, Some(1));
        assert_eq!(reste.position, 42.0);
        // « c » (en cours) et « d » ont disparu, pas de suivant : nulle part.
        let reste = reprise
            .clone()
            .sans_les_disparus(|m| m != Path::new("c") && m != Path::new("d"));
        assert_eq!(reste.morceaux, liste(&["a", "b"]));
        assert_eq!(reste.courant, None);
        assert_eq!(reste.position, 0.0);
    }

    #[test]
    fn si_le_morceau_en_cours_a_disparu_on_prend_le_debut_du_suivant() {
        let reprise = Reprise {
            morceaux: liste(&["a", "b", "c", "d"]),
            courant: Some(1),
            position: 42.0,
            repetition: true,
            ..Reprise::default()
        };
        let reste = reprise.sans_les_disparus(|m| m != Path::new("b") && m != Path::new("c"));
        assert_eq!(reste.morceaux, liste(&["a", "d"]));
        assert_eq!(reste.courant, Some(1)); // « d »
        assert_eq!(reste.position, 0.0);
        assert!(reste.repetition);
    }
}
