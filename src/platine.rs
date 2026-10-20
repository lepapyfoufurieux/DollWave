//! La « platine » : elle joue un morceau, à une vitesse
//! qui peut changer et même devenir négative (lecture à l'envers). C'est ce qui
//! permet d'imiter un scratch de vinyle quand on fait glisser la barre de progression.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::Source;

use crate::morceau::{Lecture, Morceau};

/// Pendant un scratch, temps (en secondes) que met la musique à rattraper la souris.
/// Plus c'est petit, plus le son colle au mouvement, mais plus il est brutal.
const RATTRAPAGE: f64 = 0.05;
/// Pendant un scratch, temps (en secondes) que met la vitesse à changer, comme
/// l'inertie d'un vrai disque. Évite les claquements quand la souris saute.
const INERTIE: f64 = 0.01;
/// Vitesse maximale du scratch (8 = huit fois plus vite que la normale).
const VITESSE_MAX: f64 = 8.0;
/// Pendant un freinage, vitesse en dessous de laquelle le son s'éteint peu à
/// peu : sans ça, couper net le dernier échantillon ferait un petit « clic ».
const VITESSE_FONDU: f64 = 0.2;

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
    /// Pendant un freinage : de combien la vitesse baisse à chaque trame.
    freinage: Option<f64>,
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
            lecture: Lecture::default(),
            morceau: morceau.clone(),
            etat: etat.clone(),
        };
        (Self { morceau, etat }, platine)
    }

    /// Une nouvelle platine qui reprend le morceau là où en est celle-ci
    /// (même position, en pause si elle l'était), et sa télécommande. Sert
    /// quand la sortie son change : l'ancienne platine reste coincée dans
    /// l'ancienne sortie.
    pub fn reprise(&self) -> (Self, Platine) {
        let (telecommande, platine) = Self::nouvelle(self.morceau.clone());
        {
            let ancien = self.etat.lock().unwrap();
            let mut nouveau = telecommande.etat.lock().unwrap();
            nouveau.position = ancien.position;
            nouveau.en_pause = ancien.en_pause;
        }
        (telecommande, platine)
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

    /// Freine comme un vinyle qu'on arrête à la main : la vitesse baisse
    /// régulièrement jusqu'à 0 en `duree`, le son descend dans les graves,
    /// puis la platine s'arrête (comme à la fin du morceau).
    pub fn freiner(&self, duree: Duration) {
        let pente = 1.0 / (self.morceau.frequence as f64 * duree.as_secs_f64()).max(1.0);
        let mut etat = self.etat.lock().unwrap();
        etat.freinage = Some(pente);
        etat.en_pause = false;
        etat.cible_scratch = None;
    }

    /// La platine est-elle en train de freiner (après un Stop) ?
    pub fn freine(&self) -> bool {
        self.etat.lock().unwrap().freinage.is_some()
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
    /// Le bloc du morceau en cours de lecture, gardé sous la main.
    lecture: Lecture,
}

impl Platine {
    /// Calcule la prochaine trame et avance dans le morceau.
    /// Renvoie `false` quand le morceau est terminé.
    fn preparer_trame(&mut self) -> bool {
        // On copie les ordres sans garder le verrou : lire la trame peut
        // demander de décoder un bloc (juste après un saut), et pendant ce
        // temps l'interface doit pouvoir lire la position sans attendre.
        let (position, en_pause, cible_scratch, freinage) = {
            let etat = self.etat.lock().unwrap();
            (
                etat.position,
                etat.en_pause,
                etat.cible_scratch,
                etat.freinage,
            )
        };
        let frequence = self.morceau.frequence as f64;
        let fin = self.morceau.derniere_trame();
        // Volume de la trame : 1, sauf à la toute fin d'un freinage.
        let mut gain = 1.0;

        match (freinage, cible_scratch) {
            (Some(pente), _) => {
                // Le freinage l'emporte sur tout le reste (pause, scratch).
                // Il part de la vitesse du moment (1 en lecture normale).
                self.vitesse = self.vitesse.min(1.0) - pente;
                if self.vitesse <= 0.0 || position >= fin {
                    self.vitesse = 0.0;
                    return false; // disque arrêté
                }
                gain = (self.vitesse / VITESSE_FONDU).min(1.0) as f32;
            }
            (None, Some(cible)) => {
                // On vise la vitesse qui ramène la musique sous la souris,
                // et on s'en approche en douceur (inertie du disque).
                let voulue = ((cible - position) / (frequence * RATTRAPAGE))
                    .clamp(-VITESSE_MAX, VITESSE_MAX);
                self.vitesse += (voulue - self.vitesse) / (frequence * INERTIE);
            }
            (None, None) if en_pause => {
                // En pause : du silence, sans avancer.
                self.trame.fill(0.0);
                return true;
            }
            (None, None) => {
                if position >= fin {
                    return false; // morceau terminé
                }
                self.vitesse = 1.0;
            }
        }

        // La position tombe souvent entre deux trames : on mélange les deux
        // (interpolation), sinon le son serait granuleux pendant le scratch.
        let avant = position.floor() as usize;
        let apres = (avant + 1).min(fin as usize);
        let melange = (position - avant as f64) as f32;
        let morceau = &*self.morceau;
        self.trame.fill(0.0);
        if let Some(trame) = self.lecture.trame_jouee(morceau, avant) {
            for (sortie, &a) in self.trame.iter_mut().zip(trame) {
                *sortie = a as f32;
            }
        }
        if let Some(trame) = self.lecture.trame_jouee(morceau, apres) {
            for (sortie, &b) in self.trame.iter_mut().zip(trame) {
                *sortie += (b as f32 - *sortie) * melange;
            }
        }
        for sortie in &mut self.trame {
            *sortie *= gain / 32768.0;
        }

        // Si l'interface a sauté ailleurs pendant ce temps, son saut l'emporte.
        let mut etat = self.etat.lock().unwrap();
        if etat.position == position {
            etat.position = (position + self.vitesse).clamp(0.0, fin);
        }
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
        Arc::new(Morceau::brut(
            (0..secondes * 1000).map(|i| i as i16).collect(),
            1,
            1000,
        ))
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
    fn la_vitesse_du_scratch_est_limitee() {
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau_test(60));
        telecommande.scratcher(59.0);
        for _ in 0..1000 {
            platine.next();
        }
        // En 1 seconde, impossible d'avancer de plus de 8 secondes.
        assert!(telecommande.position().as_secs_f64() <= VITESSE_MAX);
    }

    /// La reprise sur une autre sortie repart au même endroit, garde la
    /// pause, et ne dépend plus de l'ancienne platine.
    #[test]
    fn la_reprise_continue_au_meme_endroit() {
        let (telecommande, mut ancienne) = Telecommande::nouvelle(morceau_test(10));
        telecommande.aller_a(3.0);
        for _ in 0..250 {
            ancienne.next();
        }
        telecommande.mettre_en_pause(true);
        let (reprise, mut platine) = telecommande.reprise();
        assert_eq!(reprise.position(), Duration::from_millis(3250));
        assert!(reprise.en_pause());
        assert_eq!(platine.next(), Some(0.0)); // en pause : silence
        reprise.mettre_en_pause(false);
        assert_eq!(platine.next(), Some(3250.0 / 32768.0));
        // L'ancienne platine ne fait plus avancer la nouvelle.
        telecommande.mettre_en_pause(false);
        for _ in 0..100 {
            ancienne.next();
        }
        assert_eq!(reprise.position(), Duration::from_millis(3251));
    }

    /// Pendant que la platine prépare une trame (sans tenir le verrou),
    /// l'interface peut lire la position ; et un saut demandé pendant ce
    /// temps n'est pas écrasé.
    #[test]
    fn la_platine_ne_bloque_pas_l_interface_et_respecte_les_sauts() {
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau_test(10));
        telecommande.aller_a(2.0);
        for _ in 0..100 {
            platine.next();
        }
        assert_eq!(telecommande.position(), Duration::from_millis(2100));
        telecommande.aller_a(7.0);
        platine.next();
        assert_eq!(telecommande.position(), Duration::from_millis(7001));
        assert_eq!(platine.next(), Some(7001.0 / 32768.0));
    }

    /// Le freinage dure le temps demandé, la vitesse baisse régulièrement
    /// (donc le son descend dans les graves), puis la platine s'arrête.
    #[test]
    fn le_freinage_ralentit_regulierement_puis_s_arrete() {
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau_test(10));
        telecommande.aller_a(2.0);
        platine.next();
        assert!(!telecommande.freine());
        telecommande.freiner(Duration::from_secs(1));
        assert!(telecommande.freine());

        // On note l'avancée de la position à chaque trame (= la vitesse).
        let mut vitesses = Vec::new();
        let mut avant = telecommande.position().as_secs_f64();
        while platine.next().is_some() {
            let apres = telecommande.position().as_secs_f64();
            vitesses.push((apres - avant) * 1000.0);
            avant = apres;
            assert!(vitesses.len() < 2000, "ne s'arrête jamais");
        }
        // Environ 1 seconde de son (1000 trames à 1000 Hz)…
        assert!(
            (995..=1000).contains(&vitesses.len()),
            "{} trames",
            vitesses.len()
        );
        // …où la vitesse ne fait que baisser, de 1 vers 0, sans à-coup…
        assert!(
            vitesses[0] > 0.99 && vitesses[0] <= 1.0,
            "départ {}",
            vitesses[0]
        );
        assert!(*vitesses.last().unwrap() < 0.01);
        assert!(vitesses
            .windows(2)
            .all(|v| v[1] < v[0] && v[0] - v[1] < 0.0011));
        // …à mi-chemin, on est à mi-vitesse (la baisse est régulière)…
        assert!(
            (vitesses[500] - 0.5).abs() < 0.01,
            "mi-chemin {}",
            vitesses[500]
        );
        // …et le disque a tourné d'une demi-seconde de musique.
        let position = telecommande.position().as_secs_f64();
        assert!((position - 2.5).abs() < 0.01, "position {position}");
        // Une fois arrêtée, la platine ne joue plus rien.
        assert_eq!(platine.next(), None);
    }

    /// À la fin du freinage, le son s'éteint en douceur (pas de « clic ») :
    /// le dernier échantillon est presque muet.
    #[test]
    fn le_freinage_finit_en_silence() {
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau_test(10));
        telecommande.aller_a(5.0); // échantillons autour de 5000 : bien audibles
        telecommande.freiner(Duration::from_secs(1));
        let echantillons: Vec<f32> = platine.by_ref().collect();
        let plein = 5000.0 / 32768.0;
        assert!(echantillons[0] > 0.99 * plein);
        assert!(*echantillons.last().unwrap() < 0.01 * plein);
        // Le volume ne baisse qu'à la toute fin.
        assert!(echantillons[700] > 0.99 * plein);
    }

    /// Un freinage en pause ou pendant un scratch part quand même et s'arrête,
    /// et un freinage près de la fin du morceau s'arrête à la fin.
    #[test]
    fn le_freinage_l_emporte_sur_la_pause_le_scratch_et_la_fin() {
        let (telecommande, mut platine) = Telecommande::nouvelle(morceau_test(10));
        telecommande.mettre_en_pause(true);
        telecommande.freiner(Duration::from_secs(1));
        assert!(!telecommande.en_pause());
        telecommande.scratcher(9.0); // ignoré pendant le freinage
        assert!((0..2000).any(|_| platine.next().is_none()));
        assert!(telecommande.position() < Duration::from_millis(600));

        let (telecommande, platine) = Telecommande::nouvelle(morceau_test(10));
        telecommande.aller_a(9.8);
        telecommande.freiner(Duration::from_secs(1));
        assert!(platine.count() < 300);
        assert_eq!(telecommande.position(), Duration::from_millis(9999));
    }
}
