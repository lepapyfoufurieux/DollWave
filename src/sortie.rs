//! La sortie son : le lien entre l'appli et les haut-parleurs (ou le casque).
//!
//! Windows peut la couper sous nos pieds : mise en veille, casque débranché…
//! rodio ne prévient pas (il écrit juste l'erreur dans la console) et le son
//! ne repart jamais. On surveille donc qu'elle tourne toujours, grâce à un
//! « témoin » : une source de silence mélangée à la musique, qui compte
//! chaque échantillon que la sortie lui demande. Si le compte ne bouge plus,
//! la sortie est morte et l'appli en ouvre une nouvelle.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rodio::cpal::traits::HostTrait;
use rodio::{cpal, DeviceTrait, OutputStream, OutputStreamHandle, Source};

/// Au-delà de ce temps sans que la sortie demande de son, elle est considérée morte.
pub const SILENCE_MAX: Duration = Duration::from_secs(1);
/// Tous les combien on regarde si la sortie par défaut de Windows a changé
/// (ou on réessaie d'en ouvrir une, s'il n'y en avait pas).
pub const VERIFICATION: Duration = Duration::from_secs(2);

/// Une sortie son ouverte sur un périphérique.
pub struct Sortie {
    // Il faut garder le flux en vie : s'il disparaît, le son s'arrête.
    _flux: OutputStream,
    pub handle: OutputStreamHandle,
    /// Le nom du périphérique (« Haut-parleurs (Realtek…) »), pour voir si
    /// Windows en a choisi un autre.
    pub nom: Option<String>,
    /// Le compte du témoin.
    battements: Arc<AtomicU64>,
}

impl Sortie {
    /// Ouvre la sortie son par défaut de Windows.
    pub fn ouvrir() -> Result<Self, String> {
        let peripherique = cpal::default_host()
            .default_output_device()
            .ok_or("aucun périphérique de sortie")?;
        let nom = peripherique.name().ok();
        // Si le périphérique par défaut refuse, rodio essaie les autres.
        let (flux, handle) = OutputStream::try_from_device(&peripherique)
            .or_else(|_| OutputStream::try_default())
            .map_err(|e| e.to_string())?;
        let battements = Arc::new(AtomicU64::new(0));
        let temoin = Temoin {
            battements: battements.clone(),
        };
        handle.play_raw(temoin).map_err(|e| e.to_string())?;
        Ok(Self {
            _flux: flux,
            handle,
            nom,
            battements,
        })
    }

    /// Le nombre d'échantillons que la sortie a demandés au témoin.
    pub fn battements(&self) -> u64 {
        self.battements.load(Ordering::Relaxed)
    }

    /// Pour les tests : fait comme si la sortie était morte (le compte ne bouge plus).
    #[cfg(test)]
    pub fn figer(&mut self) {
        self.battements = Arc::new(AtomicU64::new(0));
    }
}

/// Le nom de la sortie par défaut de Windows en ce moment, s'il y en a une.
pub fn nom_par_defaut() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|peripherique| peripherique.name().ok())
}

/// Une source de silence sans fin, qui compte les échantillons qu'on lui demande.
struct Temoin {
    battements: Arc<AtomicU64>,
}

impl Iterator for Temoin {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        self.battements.fetch_add(1, Ordering::Relaxed);
        Some(0.0)
    }
}

impl Source for Temoin {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        48_000
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Regarde le compte du témoin de temps en temps, et dit quand il ne bouge plus.
pub struct Veilleur {
    dernier: u64,
    depuis: Instant,
}

impl Veilleur {
    pub fn new(maintenant: Instant) -> Self {
        Self {
            dernier: 0,
            depuis: maintenant,
        }
    }

    /// Vrai si le compte n'a pas bougé depuis au moins `SILENCE_MAX`.
    pub fn muette(&mut self, battements: u64, maintenant: Instant) -> bool {
        if battements != self.dernier {
            self.dernier = battements;
            self.depuis = maintenant;
            return false;
        }
        maintenant.saturating_duration_since(self.depuis) >= SILENCE_MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_temoin_compte_et_ne_fait_pas_de_bruit() {
        let battements = Arc::new(AtomicU64::new(0));
        let temoin = Temoin {
            battements: battements.clone(),
        };
        assert!(temoin.take(1000).all(|e| e == 0.0));
        assert_eq!(battements.load(Ordering::Relaxed), 1000);
    }

    #[test]
    fn une_sortie_qui_tourne_n_est_jamais_muette() {
        let debut = Instant::now();
        let mut veilleur = Veilleur::new(debut);
        for i in 1..100 {
            let maintenant = debut + Duration::from_millis(500) * i;
            assert!(!veilleur.muette(i as u64 * 24_000, maintenant));
        }
    }

    #[test]
    fn une_sortie_figee_est_muette_au_bout_d_une_seconde() {
        let debut = Instant::now();
        let mut veilleur = Veilleur::new(debut);
        assert!(!veilleur.muette(5000, debut));
        assert!(!veilleur.muette(5000, debut + Duration::from_millis(900)));
        assert!(veilleur.muette(5000, debut + SILENCE_MAX));
        // Elle repart : plus muette.
        assert!(!veilleur.muette(5001, debut + Duration::from_secs(3)));
    }

    /// Une sortie qui n'a jamais rien demandé (morte dès le départ) est
    /// aussi repérée.
    #[test]
    fn une_sortie_morte_des_le_depart_est_muette() {
        let debut = Instant::now();
        let mut veilleur = Veilleur::new(debut);
        assert!(!veilleur.muette(0, debut + Duration::from_millis(500)));
        assert!(veilleur.muette(0, debut + SILENCE_MAX));
    }

    /// L'interface peut rester bloquée longtemps (fenêtre « Ouvrir un
    /// fichier ») : si la sortie a tourné pendant ce temps, ce n'est pas une panne.
    #[test]
    fn une_longue_absence_de_l_interface_n_est_pas_une_panne() {
        let debut = Instant::now();
        let mut veilleur = Veilleur::new(debut);
        assert!(!veilleur.muette(100, debut));
        assert!(!veilleur.muette(500_000, debut + Duration::from_secs(10)));
    }
}
