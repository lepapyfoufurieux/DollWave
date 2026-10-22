//! Se souvenir de ce qu'on n'a pas trouvé en ligne (paroles d'un morceau,
//! pochette d'un album), pour ne pas le rechercher à chaque fois. On les
//! oublie au bout de 30 jours, car les sites s'enrichissent.
//!
//! Un petit fichier dans le dossier des réglages, une ligne par chose :
//! « secondes depuis 1970 <tab> clé ». Seuls les vrais « introuvables » y
//! sont notés, jamais une erreur réseau.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::systeme;

/// Les morceaux dont on n'a pas trouvé les paroles.
pub const PAROLES: &str = "paroles_introuvables.txt";
/// Les albums (ou morceaux sans album) dont on n'a pas trouvé la pochette.
pub const POCHETTES: &str = "pochettes_introuvables.txt";
/// Au-delà, on retente.
const OUBLI: Duration = Duration::from_secs(30 * 24 * 3600);

pub struct Introuvables {
    /// Rien dans les tests : la mémoire ne s'écrit pas.
    fichier: Option<PathBuf>,
    /// Clé → quand on l'a noté (secondes depuis 1970).
    notes: HashMap<String, u64>,
}

/// L'heure, en secondes depuis 1970.
pub fn maintenant() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Introuvables {
    /// La mémoire du lecteur pour ce fichier (`PAROLES` ou `POCHETTES`).
    pub fn du_lecteur(nom: &str) -> Self {
        let fichier = systeme::fichier_de_reglage(nom);
        let texte = fichier
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        Self {
            notes: lire(&texte, maintenant()),
            fichier,
        }
    }

    /// Une mémoire vide qui ne s'écrit nulle part (pour les tests).
    #[cfg(test)]
    pub fn en_memoire() -> Self {
        Self {
            fichier: None,
            notes: HashMap::new(),
        }
    }

    /// Déjà cherché sans succès il y a moins de 30 jours ?
    pub fn deja_cherche(&self, cle: &str, maintenant: u64) -> bool {
        self.notes
            .get(&nettoyer_cle(cle))
            .is_some_and(|&quand| maintenant.saturating_sub(quand) < OUBLI.as_secs())
    }

    /// Note qu'on ne l'a pas trouvé, et l'écrit tout de suite (si le
    /// lecteur est fermé en pleine recherche, rien n'est perdu).
    pub fn noter(&mut self, cle: &str, maintenant: u64) {
        self.notes.insert(nettoyer_cle(cle), maintenant);
        let Some(fichier) = &self.fichier else {
            return;
        };
        if let Some(dossier) = fichier.parent() {
            let _ = std::fs::create_dir_all(dossier);
        }
        // Si l'écriture échoue, on retentera simplement la prochaine fois.
        let _ = std::fs::write(fichier, ecrire(&self.notes));
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.notes.len()
    }
}

/// Oublie tout ce qu'on n'avait pas trouvé, paroles et pochettes : la
/// prochaine recherche retentera tout.
pub fn tout_oublier() -> Result<(), String> {
    for nom in [PAROLES, POCHETTES] {
        let Some(fichier) = systeme::fichier_de_reglage(nom) else {
            continue;
        };
        match std::fs::remove_file(&fichier) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("Impossible d'effacer {} : {e}", fichier.display()))
            }
            _ => {}
        }
    }
    Ok(())
}

/// Une clé sur une seule ligne, sans tabulation.
fn nettoyer_cle(cle: &str) -> String {
    cle.replace(['\t', '\n', '\r'], " ")
}

/// Lit le fichier, en laissant de côté ce qui a plus de 30 jours.
fn lire(texte: &str, maintenant: u64) -> HashMap<String, u64> {
    texte
        .lines()
        .filter_map(|ligne| {
            let (quand, cle) = ligne.split_once('\t')?;
            let quand: u64 = quand.parse().ok()?;
            (maintenant.saturating_sub(quand) < OUBLI.as_secs() && !cle.is_empty())
                .then(|| (cle.to_string(), quand))
        })
        .collect()
}

fn ecrire(notes: &HashMap<String, u64>) -> String {
    let mut lignes: Vec<_> = notes
        .iter()
        .map(|(cle, quand)| format!("{quand}\t{cle}\n"))
        .collect();
    lignes.sort();
    lignes.concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOUR: u64 = 24 * 3600;

    #[test]
    fn on_saute_pendant_30_jours_puis_on_retente() {
        let mut memoire = Introuvables::en_memoire();
        let debut = 1_000 * JOUR;
        assert!(!memoire.deja_cherche("Music/A/1.mp3", debut));
        memoire.noter("Music/A/1.mp3", debut);
        assert!(memoire.deja_cherche("Music/A/1.mp3", debut + 29 * JOUR));
        assert!(!memoire.deja_cherche("Music/A/1.mp3", debut + 30 * JOUR));
        assert!(!memoire.deja_cherche("Music/A/2.mp3", debut));
    }

    #[test]
    fn le_fichier_se_relit_sans_les_vieilles_lignes() {
        let mut notes = HashMap::new();
        notes.insert("War Eternal — Arch Enemy".to_string(), 100 * JOUR);
        notes.insert("vieux".to_string(), 10 * JOUR);
        let relu = lire(&ecrire(&notes), 101 * JOUR);
        assert_eq!(relu.len(), 1);
        assert_eq!(relu["War Eternal — Arch Enemy"], 100 * JOUR);
        // Lignes abîmées : ignorées.
        assert!(lire("pas un nombre\tx\nsans tabulation\n5\t\n", 6).is_empty());
    }

    #[test]
    fn une_cle_reste_sur_une_ligne() {
        let mut memoire = Introuvables::en_memoire();
        memoire.noter("a\tb\nc", 0);
        assert!(memoire.deja_cherche("a\tb\nc", 1));
        assert_eq!(ecrire(&memoire.notes), "0\ta b c\n");
    }
}
