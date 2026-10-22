//! Ce qui est commun aux recherches en ligne (paroles et pochettes) : un
//! agent qui n'accepte que le HTTPS, et de nouveaux essais patients quand
//! un site est surchargé.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// LRCLIB et MusicBrainz demandent que chaque logiciel se présente.
const AGENT: &str = concat!(
    "dollwave/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/lepapyfoufurieux/DollWave)"
);
/// Taille maximale d'une réponse (une grande pochette tient largement).
const TAILLE_MAX: u64 = 20 * 1024 * 1024;
/// Attentes avant de réessayer quand le site est surchargé (503, 429…).
const ATTENTES_SITE_SATURE: [u64; 3] = [5, 15, 30];
/// Attentes avant de réessayer quand on n'a pas de réponse du tout.
const ATTENTES_SANS_REPONSE: [u64; 2] = [1, 2];
/// On n'attend jamais plus longtemps, même si le site le demande.
const ATTENTE_MAX: Duration = Duration::from_secs(60);

/// Pourquoi une demande a échoué.
#[derive(Debug, Clone, PartialEq)]
pub enum Echec {
    /// Le site répond, mais refuse pour l'instant : surchargé ou en
    /// maintenance (503), trop de demandes (429)…
    SiteIndisponible(String),
    /// Pas de réponse : connexion coupée, délai dépassé…
    SansReponse(String),
    /// L'adresse n'est pas en HTTPS : on refuse de l'ouvrir.
    NonSecurise(String),
}

impl fmt::Display for Echec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Echec::SiteIndisponible(texte) | Echec::SansReponse(texte) => f.write_str(texte),
            Echec::NonSecurise(adresse) => write!(f, "adresse non sécurisée refusée : {adresse}"),
        }
    }
}

/// L'agent des recherches en ligne : HTTPS seulement, même après une
/// redirection.
pub fn agent(delai_max: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(AGENT)
        .https_only(true)
        // On lit nous-mêmes le code de réponse, pour savoir combien attendre.
        .http_status_as_error(false)
        .timeout_global(Some(delai_max))
        .build()
        .into()
}

/// Combien attendre avant le prochain essai (le premier nouvel essai a le
/// numéro 0), ou rien s'il ne sert à rien de réessayer. `demande` est
/// l'attente demandée par le site (en-tête « Retry-After »).
pub fn attente(essai: usize, echec: &Echec, demande: Option<Duration>) -> Option<Duration> {
    match echec {
        Echec::SiteIndisponible(_) => {
            let prevue = Duration::from_secs(*ATTENTES_SITE_SATURE.get(essai)?);
            Some(demande.map_or(prevue, |d| d.max(prevue)).min(ATTENTE_MAX))
        }
        Echec::SansReponse(_) => ATTENTES_SANS_REPONSE
            .get(essai)
            .map(|&s| Duration::from_secs(s)),
        Echec::NonSecurise(_) => None,
    }
}

/// Télécharge une adresse. Une page introuvable veut dire « rien ». En cas
/// d'échec on réessaie (voir `attente`) ; si on clique sur « Arrêter »
/// pendant l'attente, on abandonne tout de suite.
pub fn telecharger(
    agent: &ureq::Agent,
    adresse: &str,
    parametres: &[(&str, String)],
    arreter: &AtomicBool,
) -> Result<Option<Vec<u8>>, Echec> {
    let mut essai = 0;
    loop {
        let (echec, demande) = match une_fois(agent, adresse, parametres) {
            Ok(reponse) => return Ok(reponse),
            Err(erreur) => erreur,
        };
        let Some(pause) = attente(essai, &echec, demande) else {
            return Err(echec);
        };
        let fin = Instant::now() + pause;
        while Instant::now() < fin {
            if arreter.load(Ordering::Relaxed) {
                return Err(echec);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        essai += 1;
    }
}

/// Une seule demande. En cas d'échec, rend aussi l'attente demandée par le
/// site, s'il en indique une.
fn une_fois(
    agent: &ureq::Agent,
    adresse: &str,
    parametres: &[(&str, String)],
) -> Result<Option<Vec<u8>>, (Echec, Option<Duration>)> {
    let mut requete = agent.get(adresse);
    for (cle, valeur) in parametres {
        requete = requete.query(*cle, valeur);
    }
    let mut reponse = match requete.call() {
        Ok(reponse) => reponse,
        Err(ureq::Error::RequireHttpsOnly(_)) => {
            return Err((Echec::NonSecurise(adresse.to_string()), None))
        }
        Err(erreur) => return Err((Echec::SansReponse(erreur.to_string()), None)),
    };
    let code = reponse.status().as_u16();
    match code {
        200..=299 => reponse
            .body_mut()
            .with_config()
            .limit(TAILLE_MAX)
            .read_to_vec()
            .map(Some)
            .map_err(|e| (Echec::SansReponse(e.to_string()), None)),
        404 => Ok(None),
        _ => {
            let demande = reponse
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(lire_retry_after);
            Err((
                Echec::SiteIndisponible(format!("http status: {code}")),
                demande,
            ))
        }
    }
}

/// « Retry-After: 120 » : un nombre de secondes. (La forme avec une date
/// est rare ; on l'ignore et on garde nos attentes.)
fn lire_retry_after(valeur: &str) -> Option<Duration> {
    valeur.trim().parse().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sature() -> Echec {
        Echec::SiteIndisponible("http status: 503".to_string())
    }

    #[test]
    fn un_site_sature_laisse_le_temps_de_souffler() {
        let attentes: Vec<_> = (0..4)
            .map(|essai| attente(essai, &sature(), None))
            .collect();
        assert_eq!(
            attentes,
            [5, 15, 30]
                .map(|s| Some(Duration::from_secs(s)))
                .into_iter()
                .chain([None])
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn on_respecte_l_attente_demandee_par_le_site_sans_exagerer() {
        let s = Duration::from_secs;
        assert_eq!(attente(0, &sature(), Some(s(20))), Some(s(20)));
        // Jamais moins que notre propre attente…
        assert_eq!(attente(1, &sature(), Some(s(1))), Some(s(15)));
        // … ni plus d'une minute.
        assert_eq!(attente(0, &sature(), Some(s(3600))), Some(s(60)));
        assert_eq!(lire_retry_after(" 120 "), Some(s(120)));
        assert_eq!(lire_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }

    #[test]
    fn sans_reponse_on_reessaie_vite_et_peu() {
        let echec = Echec::SansReponse("timeout".to_string());
        assert_eq!(attente(0, &echec, None), Some(Duration::from_secs(1)));
        assert_eq!(attente(1, &echec, None), Some(Duration::from_secs(2)));
        assert_eq!(attente(2, &echec, None), None);
    }

    #[test]
    fn une_adresse_http_est_refusee_sans_rien_envoyer() {
        let resultat = telecharger(
            &agent(Duration::from_secs(5)),
            "http://lrclib.net/api/search",
            &[],
            &AtomicBool::new(false),
        );
        assert_eq!(
            resultat,
            Err(Echec::NonSecurise(
                "http://lrclib.net/api/search".to_string()
            ))
        );
    }
}
