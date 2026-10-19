//! La « platine » : elle joue un morceau déjà décodé en mémoire, à une vitesse
//! qui peut changer et même devenir négative (lecture à l'envers). C'est ce qui
//! permet d'imiter un scratch de vinyle quand on fait glisser la barre de progression.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::{Decoder, Source};

/// Pendant un scratch, temps (en secondes) que met la musique à rattraper la souris.
/// Plus c'est petit, plus le son colle au mouvement, mais plus il est brutal.
const RATTRAPAGE: f64 = 0.05;
/// Pendant un scratch, temps (en secondes) que met la vitesse à changer, comme
/// l'inertie d'un vrai disque. Évite les claquements quand la souris saute.
const INERTIE: f64 = 0.01;
/// Vitesse maximale du scratch (8 = huit fois plus vite que la normale).
const VITESSE_MAX: f64 = 8.0;

/// Un morceau entièrement décodé : tous ses échantillons sonores, entrelacés
/// (gauche, droite, gauche, droite… pour un morceau en stéréo).
pub struct Morceau {
    echantillons: Vec<i16>,
    canaux: usize,
    frequence: u32,
}

impl Morceau {
    /// Décode tout le fichier MP3 d'un coup. Pour un morceau de 4 minutes,
    /// cela prend environ 40 Mo de mémoire.
    pub fn charger(chemin: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let decodeur = Decoder::new(BufReader::new(File::open(chemin)?))?;
        let canaux = decodeur.channels() as usize;
        let frequence = decodeur.sample_rate();
        let echantillons: Vec<i16> = decodeur.collect();
        if canaux == 0 || echantillons.len() < canaux {
            return Err("le fichier ne contient pas de son".into());
        }
        Ok(Self {
            echantillons,
            canaux,
            frequence,
        })
    }

    /// Un son pur (sinusoïde) mono au volume maximum, pour les tests.
    #[cfg(test)]
    pub fn sinus(hz: f32, frequence: u32, secondes: f32) -> Self {
        let n = (frequence as f32 * secondes) as usize;
        let echantillons = (0..n)
            .map(|i| {
                let t = i as f32 / frequence as f32;
                ((std::f32::consts::TAU * hz * t).sin() * 32767.0) as i16
            })
            .collect();
        Self {
            echantillons,
            canaux: 1,
            frequence,
        }
    }

    /// Nombre de « trames » : une trame = un échantillon pour chaque canal.
    fn nb_trames(&self) -> usize {
        self.echantillons.len() / self.canaux
    }

    pub fn duree(&self) -> Duration {
        self.secondes(self.nb_trames() as f64)
    }

    fn secondes(&self, trames: f64) -> Duration {
        Duration::from_secs_f64(trames / self.frequence as f64)
    }

    fn trames(&self, secondes: f32) -> f64 {
        (secondes as f64 * self.frequence as f64).clamp(0.0, self.derniere_trame())
    }

    fn derniere_trame(&self) -> f64 {
        (self.nb_trames() - 1) as f64
    }

    /// Nombre d'échantillons par seconde (par exemple 44 100).
    pub fn frequence(&self) -> u32 {
        self.frequence
    }

    /// `n` échantillons à partir de `secondes`, entre -1 et 1. Les canaux sont
    /// mélangés en un seul (mono). Après la fin du morceau : du silence.
    pub fn extrait_mono(&self, secondes: f32, n: usize) -> Vec<f32> {
        let debut = self.trames(secondes) as usize;
        let c = self.canaux;
        (debut..debut + n)
            .map(|t| match self.echantillons.get(t * c..(t + 1) * c) {
                Some(trame) => trame.iter().map(|&e| e as f32).sum::<f32>() / (c as f32 * 32768.0),
                None => 0.0,
            })
            .collect()
    }

    /// Un résumé de tout le morceau, découpé en `par_seconde` tranches par
    /// seconde : pour chaque tranche, l'échantillon le plus bas et le plus haut
    /// (entre -1 et 1).
    pub fn apercu(&self, par_seconde: u32) -> Vec<(f32, f32)> {
        let par_tranche = (self.frequence / par_seconde).max(1) as usize * self.canaux;
        self.echantillons
            .chunks(par_tranche)
            .map(|tranche| {
                let (bas, haut) = tranche
                    .iter()
                    .fold((0i16, 0i16), |(b, h), &e| (b.min(e), h.max(e)));
                (bas as f32 / 32768.0, haut as f32 / 32768.0)
            })
            .collect()
    }
}

/// Ce que l'interface et la platine partagent : l'interface écrit les ordres,
/// la platine (qui tourne dans le fil audio) les lit et met à jour la position.
#[derive(Default)]
struct Etat {
    /// Où on en est dans le morceau, en trames (avec une partie décimale,
    /// car pendant un scratch on avance de fractions de trame).
    position: f64,
    en_pause: bool,
    /// Pendant un scratch : l'endroit du morceau où se trouve la souris.
    cible_scratch: Option<f64>,
}

/// La télécommande de la platine, gardée par l'interface.
#[derive(Clone)]
pub struct Telecommande {
    morceau: Arc<Morceau>,
    etat: Arc<Mutex<Etat>>,
}

impl Telecommande {
    /// Crée une platine prête à jouer le morceau depuis le début, et sa télécommande.
    pub fn nouvelle(morceau: Arc<Morceau>) -> (Self, Platine) {
        let etat = Arc::new(Mutex::new(Etat::default()));
        let platine = Platine {
            trame: vec![0.0; morceau.canaux],
            canal: 0,
            vitesse: 1.0,
            morceau: morceau.clone(),
            etat: etat.clone(),
        };
        (Self { morceau, etat }, platine)
    }

    pub fn position(&self) -> Duration {
        self.morceau.secondes(self.etat.lock().unwrap().position)
    }

    pub fn en_pause(&self) -> bool {
        self.etat.lock().unwrap().en_pause
    }

    pub fn mettre_en_pause(&self, pause: bool) {
        self.etat.lock().unwrap().en_pause = pause;
    }

    /// Saute directement à un endroit du morceau (sans bruit de scratch).
    pub fn aller_a(&self, secondes: f32) {
        let mut etat = self.etat.lock().unwrap();
        etat.position = self.morceau.trames(secondes);
        etat.cible_scratch = None;
    }

    /// Pendant un glissement : indique où se trouve la souris. La musique la suit.
    pub fn scratcher(&self, secondes: f32) {
        self.etat.lock().unwrap().cible_scratch = Some(self.morceau.trames(secondes));
    }
}

/// La source sonore donnée à rodio. rodio lui demande les échantillons un par un.
pub struct Platine {
    morceau: Arc<Morceau>,
    etat: Arc<Mutex<Etat>>,
    /// Vitesse actuelle (1 = normale, 0 = arrêt, négative = à l'envers).
    vitesse: f64,
    /// La trame en cours d'envoi, et le prochain canal à envoyer.
    trame: Vec<f32>,
    canal: usize,
}

impl Platine {
    /// Calcule la prochaine trame et avance dans le morceau.
    /// Renvoie `false` quand le morceau est terminé.
    fn preparer_trame(&mut self) -> bool {
        let mut etat = self.etat.lock().unwrap();
        let frequence = self.morceau.frequence as f64;
        let fin = self.morceau.derniere_trame();

        match etat.cible_scratch {
            Some(cible) => {
                // On vise la vitesse qui ramène la musique sous la souris,
                // et on s'en approche en douceur (inertie du disque).
                let voulue = ((cible - etat.position) / (frequence * RATTRAPAGE))
                    .clamp(-VITESSE_MAX, VITESSE_MAX);
                self.vitesse += (voulue - self.vitesse) / (frequence * INERTIE);
            }
            None if etat.en_pause => {
                // En pause : du silence, sans avancer.
                self.trame.fill(0.0);
                return true;
            }
            None => {
                if etat.position >= fin {
                    return false; // morceau terminé
                }
                self.vitesse = 1.0;
            }
        }

        // La position tombe souvent entre deux trames : on mélange les deux
        // (interpolation), sinon le son serait granuleux pendant le scratch.
        let avant = etat.position.floor() as usize;
        let apres = (avant + 1).min(fin as usize);
        let melange = (etat.position - avant as f64) as f32;
        let n = self.morceau.canaux;
        for (canal, sortie) in self.trame.iter_mut().enumerate() {
            let a = self.morceau.echantillons[avant * n + canal] as f32;
            let b = self.morceau.echantillons[apres * n + canal] as f32;
            *sortie = (a + (b - a) * melange) / 32768.0;
        }

        etat.position = (etat.position + self.vitesse).clamp(0.0, fin);
        true
    }
}

impl Iterator for Platine {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.canal == 0 && !self.preparer_trame() {
            return None;
        }
        let echantillon = self.trame[self.canal];
        self.canal = (self.canal + 1) % self.trame.len();
        Some(echantillon)
    }
}

impl Source for Platine {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.morceau.canaux as u16
    }

    fn sample_rate(&self) -> u32 {
        self.morceau.frequence
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un faux morceau mono de `secondes` secondes à 1000 Hz, dont chaque
    /// échantillon vaut son numéro : on peut ainsi savoir où on en est.
    fn morceau_test(secondes: usize) -> Arc<Morceau> {
        Arc::new(Morceau {
            echantillons: (0..secondes * 1000).map(|i| i as i16).collect(),
            canaux: 1,
            frequence: 1000,
        })
    }

    #[test]
    fn lecture_normale_jusqu_a_la_fin() {
        let (telecommande, platine) = Telecommande::nouvelle(morceau_test(2));
        let echantillons: Vec<f32> = platine.collect();
        assert_eq!(echantillons.len(), 1999);
        assert_eq!(telecommande.position(), Duration::from_millis(1999));
    }

    #[test]
    fn pause_donne_du_silence_sans_avancer() {
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau_test(2));
        telecommande.aller_a(1.0);
        telecommande.mettre_en_pause(true);
        assert!((0..500).all(|_| platine.next() == Some(0.0)));
        assert_eq!(telecommande.position(), Duration::from_secs(1));
    }

    #[test]
    fn le_scratch_suit_la_souris_puis_s_arrete() {
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau_test(10));
        telecommande.aller_a(5.0);
        // La souris recule de 2 secondes et ne bouge plus.
        telecommande.scratcher(3.0);
        for _ in 0..2000 {
            platine.next();
        }
        let position = telecommande.position().as_secs_f64();
        assert!((position - 3.0).abs() < 0.01, "position {position}");
        // Souris immobile : la musique ne bouge presque plus.
        platine.next();
        let avant = telecommande.position();
        platine.next();
        let ecart = telecommande.position().abs_diff(avant);
        assert!(ecart < Duration::from_micros(100), "écart {ecart:?}");
    }

    #[test]
    fn extrait_mono_melange_les_canaux_et_finit_par_du_silence() {
        // Stéréo : gauche = 16384 (0,5), droite = 0, sur 3 trames.
        let morceau = Morceau {
            echantillons: vec![16384, 0, 16384, 0, 16384, 0],
            canaux: 2,
            frequence: 1000,
        };
        assert_eq!(
            morceau.extrait_mono(0.0, 5),
            vec![0.25, 0.25, 0.25, 0.0, 0.0]
        );
    }

    #[test]
    fn l_apercu_garde_le_plus_bas_et_le_plus_haut() {
        let morceau = Morceau {
            echantillons: vec![-16384, 8192, 0, 16384],
            canaux: 1,
            frequence: 1000,
        };
        assert_eq!(morceau.apercu(500), vec![(-0.5, 0.25), (0.0, 0.5)]);
    }

    #[test]
    fn la_vitesse_du_scratch_est_limitee() {
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau_test(60));
        telecommande.scratcher(59.0);
        for _ in 0..1000 {
            platine.next();
        }
        // En 1 seconde, impossible d'avancer de plus de 8 secondes.
        assert!(telecommande.position().as_secs_f64() <= VITESSE_MAX);
    }
}
