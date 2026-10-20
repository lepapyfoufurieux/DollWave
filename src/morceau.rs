//! Le « morceau » : un fichier MP3 prêt à être lu à n'importe quel endroit.
//!
//! On ne décode pas tout le fichier d'un coup (cela prendrait environ 40 Mo pour
//! 4 minutes et figerait la fenêtre). On garde le MP3 compressé en mémoire,
//! découpé en blocs d'environ une seconde, et on ne décode que les blocs autour
//! de l'endroit écouté. Un fil en arrière-plan prépare les blocs voisins à
//! l'avance, pour que le scratch reste fluide.

use std::collections::HashMap;
use std::fs::File;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecParameters, Decoder, DecoderOptions};
use symphonia::core::errors::Error as ErreurSymphonia;
use symphonia::core::formats::{FormatOptions, FormatReader, Packet};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::default::codecs::MpaDecoder;
use symphonia::default::formats::MpaReader;

/// Durée visée pour un bloc, en secondes.
const SECONDES_PAR_BLOC: f64 = 1.0;
/// Pour décoder un bloc, on commence quelques paquets plus tôt : un paquet MP3
/// a besoin de la fin des précédents pour être décodé correctement.
const AMORCE: usize = 8;
/// Nombre de blocs que le fil d'arrière-plan prépare de chaque côté de l'endroit écouté.
const A_PREPARER: usize = 8;
/// Les blocs plus loin que ça de l'endroit écouté sont oubliés (au plus
/// 31 blocs gardés, soit environ 5 Mo pour un morceau en stéréo).
const A_GARDER: usize = 15;
/// Quand tout est prêt, le fil d'arrière-plan dort jusqu'à ce que la platine
/// change de bloc. Il se réveille quand même de temps en temps pour voir si
/// le morceau sert encore (sinon, il s'arrête).
const PAUSE_PREPARATION: Duration = Duration::from_secs(1);

/// Ce dont on tire le son.
enum Contenu {
    /// Les paquets du fichier MP3, encore compressés.
    Mp3 {
        paquets: Vec<Packet>,
        parametres: CodecParameters,
        /// Pour chaque bloc, les numéros de ses paquets.
        paquets_des_blocs: Vec<Range<usize>>,
    },
    /// Des échantillons déjà décodés (pour les tests).
    #[cfg(test)]
    Brut(Vec<i16>),
}

pub struct Morceau {
    pub(crate) canaux: usize,
    pub(crate) frequence: u32,
    nb_trames: usize,
    /// La première trame de chaque bloc.
    debuts: Vec<usize>,
    contenu: Contenu,
    /// Les blocs déjà décodés, rangés par numéro. Chacun contient ses
    /// échantillons entrelacés (gauche, droite, gauche, droite…).
    blocs: Mutex<HashMap<usize, Arc<[i16]>>>,
    /// Le bloc que la platine est en train de jouer.
    centre: AtomicUsize,
    /// Pour réveiller le fil d'arrière-plan quand la platine change de bloc.
    reveil: Arc<Reveil>,
}

/// Le réveil du fil d'arrière-plan : `a_faire` passe à vrai quand il y a
/// peut-être de nouveaux blocs à préparer.
#[derive(Default)]
struct Reveil {
    a_faire: Mutex<bool>,
    signal: Condvar,
}

impl Reveil {
    fn sonner(&self) {
        *self.a_faire.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.signal.notify_one();
    }

    /// Attend une sonnerie (au plus `PAUSE_PREPARATION`).
    fn attendre(&self) {
        let a_faire = self.a_faire.lock().unwrap_or_else(|e| e.into_inner());
        let (mut a_faire, _) = self
            .signal
            .wait_timeout_while(a_faire, PAUSE_PREPARATION, |a_faire| !*a_faire)
            .unwrap_or_else(|e| e.into_inner());
        *a_faire = false;
    }
}

impl Morceau {
    /// Prépare un fichier MP3 : lit ses paquets (sans les décoder) et lance le
    /// fil qui décode les blocs à l'avance.
    pub fn charger(chemin: &Path) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let morceau = Arc::new(Self::depuis(Box::new(File::open(chemin)?))?);
        morceau.lancer_la_preparation();
        Ok(morceau)
    }

    /// Lance le fil qui prépare les blocs à l'avance.
    fn lancer_la_preparation(self: &Arc<Self>) {
        let suivi = Arc::downgrade(self);
        let reveil = self.reveil.clone();
        thread::spawn(move || preparer_en_continu(suivi, &reveil));
    }

    /// Lit tous les paquets d'un MP3 et les regroupe en blocs.
    fn depuis(source: Box<dyn MediaSource>) -> Result<Self, Box<dyn std::error::Error>> {
        let flux = MediaSourceStream::new(source, Default::default());
        let options = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let mut lecteur = MpaReader::try_new(flux, &options)?;
        let parametres = lecteur
            .default_track()
            .ok_or("le fichier ne contient pas de son")?
            .codec_params
            .clone();
        let canaux = parametres.channels.map_or(0, |c| c.count());
        let frequence = parametres.sample_rate.unwrap_or(0);
        if canaux == 0 || frequence == 0 {
            return Err("le fichier ne contient pas de son".into());
        }

        let mut paquets = Vec::new();
        loop {
            match lecteur.next_packet() {
                Ok(paquet) => paquets.push(paquet),
                // Fin du fichier (ou fichier abîmé à la fin) : on garde ce qu'on a.
                Err(ErreurSymphonia::IoError(_)) => break,
                Err(e) => return Err(e.into()),
            }
        }

        // On regroupe les paquets en blocs d'environ une seconde.
        let trames_par_bloc = (frequence as f64 * SECONDES_PAR_BLOC) as usize;
        let mut debuts = Vec::new();
        let mut paquets_des_blocs = Vec::new();
        let mut nb_trames = 0;
        for (n, paquet) in paquets.iter().enumerate() {
            if debuts
                .last()
                .is_none_or(|&debut| nb_trames - debut >= trames_par_bloc)
            {
                debuts.push(nb_trames);
                paquets_des_blocs.push(n..n);
            }
            paquets_des_blocs.last_mut().unwrap().end = n + 1;
            nb_trames += paquet.dur as usize;
        }
        if nb_trames < 1 {
            return Err("le fichier ne contient pas de son".into());
        }

        Ok(Self::nouveau(
            canaux,
            frequence,
            nb_trames,
            debuts,
            Contenu::Mp3 {
                paquets,
                parametres,
                paquets_des_blocs,
            },
        ))
    }

    fn nouveau(
        canaux: usize,
        frequence: u32,
        nb_trames: usize,
        debuts: Vec<usize>,
        contenu: Contenu,
    ) -> Self {
        Self {
            canaux,
            frequence,
            nb_trames,
            debuts,
            contenu,
            blocs: Mutex::new(HashMap::new()),
            centre: AtomicUsize::new(0),
            reveil: Arc::default(),
        }
    }

    /// Un morceau déjà décodé, découpé en blocs d'une seconde (pour les tests).
    #[cfg(test)]
    pub fn brut(echantillons: Vec<i16>, canaux: usize, frequence: u32) -> Self {
        let nb_trames = echantillons.len() / canaux;
        let debuts = (0..nb_trames).step_by(frequence as usize).collect();
        Self::nouveau(
            canaux,
            frequence,
            nb_trames,
            debuts,
            Contenu::Brut(echantillons),
        )
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
        Self::brut(echantillons, 1, frequence)
    }

    /// Nombre de « trames » : une trame = un échantillon pour chaque canal.
    fn nb_trames(&self) -> usize {
        self.nb_trames
    }

    pub fn duree(&self) -> Duration {
        self.secondes(self.nb_trames() as f64)
    }

    pub(crate) fn secondes(&self, trames: f64) -> Duration {
        Duration::from_secs_f64(trames / self.frequence as f64)
    }

    pub(crate) fn trames(&self, secondes: f32) -> f64 {
        (secondes as f64 * self.frequence as f64).clamp(0.0, self.derniere_trame())
    }

    pub(crate) fn derniere_trame(&self) -> f64 {
        (self.nb_trames() - 1) as f64
    }

    /// Nombre d'échantillons par seconde (par exemple 44 100).
    pub fn frequence(&self) -> u32 {
        self.frequence
    }

    /// Le numéro du bloc qui contient la trame `trame`.
    fn bloc_de(&self, trame: usize) -> usize {
        self.debuts
            .partition_point(|&debut| debut <= trame)
            .saturating_sub(1)
    }

    /// Les trames couvertes par le bloc numéro `n`.
    fn trames_du_bloc(&self, n: usize) -> Range<usize> {
        let fin = self.debuts.get(n + 1).copied().unwrap_or(self.nb_trames);
        self.debuts[n]..fin
    }

    /// Indique quel bloc la platine joue : le fil d'arrière-plan prépare ses
    /// voisins, et les blocs trop loin sont oubliés.
    fn suivre(&self, n: usize) {
        if self.centre.swap(n, Ordering::Relaxed) != n {
            self.reveil.sonner();
        }
    }

    /// Les échantillons du bloc numéro `n`, décodés si besoin.
    fn bloc(&self, n: usize) -> Arc<[i16]> {
        if let Some(bloc) = self.blocs.lock().unwrap().get(&n) {
            return bloc.clone();
        }
        // On décode sans bloquer les autres (la platine peut continuer à jouer
        // un bloc déjà prêt pendant ce temps).
        let bloc: Arc<[i16]> = self.decoder(n).into();
        let mut blocs = self.blocs.lock().unwrap();
        let centre = self.centre.load(Ordering::Relaxed);
        blocs.retain(|&m, _| m.abs_diff(centre) <= A_GARDER);
        blocs.insert(n, bloc.clone());
        bloc
    }

    /// Décode le bloc numéro `n`. Le résultat a toujours la bonne longueur
    /// (complété par du silence si un paquet est abîmé).
    fn decoder(&self, n: usize) -> Vec<i16> {
        let trames = self.trames_du_bloc(n);
        let longueur = trames.len() * self.canaux;
        let mut echantillons = match &self.contenu {
            Contenu::Mp3 {
                paquets,
                parametres,
                paquets_des_blocs,
            } => decoder_paquets(
                paquets,
                parametres,
                paquets_des_blocs[n].clone(),
                self.canaux,
            ),
            #[cfg(test)]
            Contenu::Brut(tout) => {
                tout[trames.start * self.canaux..trames.end * self.canaux].to_vec()
            }
        };
        echantillons.resize(longueur, 0);
        echantillons
    }

    /// Décode à l'avance les blocs autour de celui que joue la platine, en
    /// commençant par les plus proches. Renvoie `false` s'il n'y avait rien à faire.
    fn preparer_un_bloc(&self) -> bool {
        let centre = self.centre.load(Ordering::Relaxed);
        let deja_prets = self.blocs.lock().unwrap();
        let a_faire = (0..=A_PREPARER)
            .flat_map(|ecart| [centre.checked_add(ecart), centre.checked_sub(ecart)])
            .flatten()
            .find(|n| *n < self.debuts.len() && !deja_prets.contains_key(n));
        drop(deja_prets);
        match a_faire {
            Some(n) => {
                self.bloc(n);
                true
            }
            None => false,
        }
    }

    /// `n` échantillons à partir de `secondes`, entre -1 et 1. Les canaux sont
    /// mélangés en un seul (mono). Après la fin du morceau : du silence.
    pub fn extrait_mono(&self, secondes: f32, n: usize) -> Vec<f32> {
        let debut = self.trames(secondes) as usize;
        let mut lecture = Lecture::default();
        (debut..debut + n)
            .map(|t| match lecture.trame(self, t) {
                Some(trame) => {
                    trame.iter().map(|&e| e as f32).sum::<f32>() / (self.canaux as f32 * 32768.0)
                }
                None => 0.0,
            })
            .collect()
    }

    /// Un résumé de tout le morceau, découpé en `par_seconde` tranches par
    /// seconde : pour chaque tranche, l'échantillon le plus bas et le plus haut
    /// (entre -1 et 1). Décode tout le morceau, mais un bloc à la fois, sans
    /// garder les blocs : à lancer en arrière-plan.
    #[cfg(test)]
    pub fn apercu(&self, par_seconde: u32) -> Vec<(f32, f32)> {
        resumer(self.par_tranche(par_seconde), self.debuts.len(), |n| {
            Some(self.decoder(n))
        })
        .unwrap_or_default()
    }

    /// Comme `apercu`, mais s'arrête (et ne renvoie rien) dès que le morceau
    /// ne sert plus : on a changé de morceau, inutile de finir le calcul.
    pub fn apercu_tant_qu_utile(suivi: &Weak<Self>, par_seconde: u32) -> Option<Vec<(f32, f32)>> {
        let (par_tranche, nb_blocs) = {
            let morceau = suivi.upgrade()?;
            (morceau.par_tranche(par_seconde), morceau.debuts.len())
        };
        resumer(par_tranche, nb_blocs, |n| Some(suivi.upgrade()?.decoder(n)))
    }

    fn par_tranche(&self, par_seconde: u32) -> usize {
        (self.frequence / par_seconde).max(1) as usize * self.canaux
    }
}

/// Le résumé (plus bas, plus haut) de chaque tranche de `par_tranche`
/// échantillons, bloc après bloc. Rien si `bloc` ne donne pas un bloc.
fn resumer(
    par_tranche: usize,
    nb_blocs: usize,
    mut bloc: impl FnMut(usize) -> Option<Vec<i16>>,
) -> Option<Vec<(f32, f32)>> {
    let mut apercu = Vec::new();
    let mut tranche = (0i16, 0i16, 0usize); // plus bas, plus haut, nombre d'échantillons
    for n in 0..nb_blocs {
        for e in bloc(n)? {
            tranche = (tranche.0.min(e), tranche.1.max(e), tranche.2 + 1);
            if tranche.2 == par_tranche {
                apercu.push((tranche.0 as f32 / 32768.0, tranche.1 as f32 / 32768.0));
                tranche = (0, 0, 0);
            }
        }
    }
    if tranche.2 > 0 {
        apercu.push((tranche.0 as f32 / 32768.0, tranche.1 as f32 / 32768.0));
    }
    Some(apercu)
}

/// Décode les paquets d'un bloc, en commençant un peu avant pour « amorcer »
/// le décodeur, et ne garde que le son du bloc.
fn decoder_paquets(
    paquets: &[Packet],
    parametres: &CodecParameters,
    bloc: Range<usize>,
    canaux: usize,
) -> Vec<i16> {
    let mut sortie = Vec::new();
    let Ok(mut decodeur) = MpaDecoder::try_new(parametres, &DecoderOptions::default()) else {
        return sortie; // le bloc restera silencieux
    };
    let mut tampon: Option<SampleBuffer<i16>> = None;
    let amorce = bloc.start.saturating_sub(AMORCE);
    for (n, paquet) in paquets.iter().enumerate().take(bloc.end).skip(amorce) {
        let decode = decodeur.decode(paquet);
        if n < bloc.start {
            continue; // amorce : on jette le son
        }
        match decode {
            Ok(son) => {
                let tampon = tampon
                    .get_or_insert_with(|| SampleBuffer::new(son.capacity() as u64, *son.spec()));
                tampon.copy_interleaved_ref(son);
                sortie.extend_from_slice(tampon.samples());
            }
            // Paquet abîmé : du silence à la place.
            Err(_) => sortie.resize(sortie.len() + paquet.dur as usize * canaux, 0),
        }
    }
    sortie
}

/// Le travail du fil d'arrière-plan : préparer les blocs autour de la platine,
/// jusqu'à ce que le morceau ne serve plus à personne.
fn preparer_en_continu(suivi: Weak<Morceau>, reveil: &Reveil) {
    loop {
        let Some(morceau) = suivi.upgrade() else {
            return; // morceau abandonné
        };
        let travail_fait = morceau.preparer_un_bloc();
        drop(morceau);
        if !travail_fait {
            // Tout est prêt : on dort jusqu'à ce que la platine bouge.
            reveil.attendre();
        }
    }
}

/// Accès rapide, trame par trame, aux blocs d'un morceau : on garde le dernier
/// bloc utilisé sous la main pour ne pas le redemander à chaque échantillon.
#[derive(Default)]
pub(crate) struct Lecture {
    bloc: Option<(Range<usize>, Arc<[i16]>)>,
}

impl Lecture {
    /// Les échantillons (un par canal) de la trame numéro `t`, ou `None` après la fin.
    pub(crate) fn trame<'a>(&'a mut self, morceau: &Morceau, t: usize) -> Option<&'a [i16]> {
        if t >= morceau.nb_trames {
            return None;
        }
        if !self.contient(t) {
            let n = morceau.bloc_de(t);
            self.bloc = Some((morceau.trames_du_bloc(n), morceau.bloc(n)));
        }
        let (trames, echantillons) = self.bloc.as_ref().unwrap();
        let c = morceau.canaux;
        let debut = (t - trames.start) * c;
        echantillons.get(debut..debut + c)
    }

    /// Comme `trame`, mais signale aussi au morceau quel bloc la platine joue.
    pub(crate) fn trame_jouee<'a>(&'a mut self, morceau: &Morceau, t: usize) -> Option<&'a [i16]> {
        if !self.contient(t) {
            morceau.suivre(morceau.bloc_de(t));
        }
        self.trame(morceau, t)
    }

    fn contient(&self, t: usize) -> bool {
        matches!(&self.bloc, Some((trames, _)) if trames.contains(&t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const GLISSANDO: &[u8] = include_bytes!("../tests/glissando.mp3");

    fn glissando() -> Morceau {
        Morceau::depuis(Box::new(Cursor::new(GLISSANDO.to_vec()))).unwrap()
    }

    /// Tout le morceau décodé d'un seul coup, du début à la fin, comme avant.
    fn decodage_complet(morceau: &Morceau) -> Vec<i16> {
        let Contenu::Mp3 {
            paquets,
            parametres,
            ..
        } = &morceau.contenu
        else {
            unreachable!()
        };
        decoder_paquets(paquets, parametres, 0..paquets.len(), morceau.canaux)
    }

    #[test]
    fn le_mp3_est_decoupe_en_blocs_d_une_seconde() {
        let morceau = glissando();
        assert_eq!(morceau.canaux, 2);
        assert_eq!(morceau.frequence, 44_100);
        // 4 secondes de son : 4 blocs (le dernier peut être un peu plus court).
        assert!((3.9..4.1).contains(&morceau.duree().as_secs_f64()));
        assert_eq!(morceau.debuts.len(), 4);
    }

    #[test]
    fn decoder_bloc_par_bloc_donne_le_meme_son_que_tout_d_un_coup() {
        let morceau = glissando();
        let complet = decodage_complet(&morceau);
        assert_eq!(complet.len(), morceau.nb_trames * 2);
        // On décode les blocs dans le désordre, comme lors d'un saut.
        for n in [2, 0, 3, 1] {
            let trames = morceau.trames_du_bloc(n);
            let attendu = &complet[trames.start * 2..trames.end * 2];
            let bloc = morceau.bloc(n);
            let ecart_max = attendu
                .iter()
                .zip(bloc.iter())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert_eq!(bloc.len(), attendu.len(), "bloc {n}");
            assert!(ecart_max <= 1, "bloc {n} : écart {ecart_max}");
        }
    }

    #[test]
    fn la_lecture_passe_d_un_bloc_a_l_autre_sans_trou() {
        // 3 secondes mono à 1000 Hz : chaque échantillon vaut son numéro.
        let morceau = Morceau::brut((0..3000).collect(), 1, 1000);
        let mut lecture = Lecture::default();
        for t in [0, 999, 1000, 1001, 2999, 1500, 10] {
            assert_eq!(lecture.trame(&morceau, t), Some(&[t as i16][..]));
        }
        assert_eq!(lecture.trame(&morceau, 3000), None);
    }

    #[test]
    fn les_blocs_loin_de_la_platine_sont_oublies() {
        let morceau = Morceau::brut(vec![0; 60_000], 1, 1000);
        for n in 0..60 {
            morceau.suivre(n);
            morceau.bloc(n);
        }
        let gardes = morceau.blocs.lock().unwrap().len();
        assert!(gardes <= 2 * A_GARDER + 1, "{gardes} blocs gardés");
    }

    #[test]
    fn le_fil_d_arriere_plan_prepare_les_voisins_en_commencant_par_le_plus_proche() {
        let morceau = Morceau::brut(vec![0; 60_000], 1, 1000);
        morceau.suivre(30);
        assert!(morceau.preparer_un_bloc());
        assert!(morceau.blocs.lock().unwrap().contains_key(&30));
        while morceau.preparer_un_bloc() {}
        let blocs = morceau.blocs.lock().unwrap();
        assert!((30 - A_PREPARER..=30 + A_PREPARER).all(|n| blocs.contains_key(&n)));
    }

    #[test]
    fn extrait_mono_melange_les_canaux_et_finit_par_du_silence() {
        // Stéréo : gauche = 16384 (0,5), droite = 0, sur 3 trames.
        let morceau = Morceau::brut(vec![16384, 0, 16384, 0, 16384, 0], 2, 1000);
        assert_eq!(
            morceau.extrait_mono(0.0, 5),
            vec![0.25, 0.25, 0.25, 0.0, 0.0]
        );
    }

    #[test]
    fn l_apercu_garde_le_plus_bas_et_le_plus_haut() {
        let morceau = Morceau::brut(vec![-16384, 8192, 0, 16384], 1, 1000);
        assert_eq!(morceau.apercu(500), vec![(-0.5, 0.25), (0.0, 0.5)]);
    }

    #[test]
    fn l_apercu_d_un_mp3_couvre_tout_le_morceau() {
        let morceau = glissando();
        let apercu = morceau.apercu(100);
        let attendu = (morceau.duree().as_secs_f64() * 100.0).ceil() as usize;
        assert_eq!(apercu.len(), attendu);
        assert!(apercu.iter().any(|&(bas, haut)| bas < -0.3 && haut > 0.3));
    }

    /// Le fil d'arrière-plan dort quand tout est prêt, et se réveille dès
    /// que la platine change de bloc (sans attendre la fin de sa pause).
    #[test]
    fn le_fil_d_arriere_plan_se_reveille_quand_la_platine_bouge() {
        let morceau = Arc::new(Morceau::sinus(440.0, 8000, 60.0));
        morceau.lancer_la_preparation();
        let pret = |n: usize| morceau.blocs.lock().unwrap().contains_key(&n);
        let attendre = |n: usize| {
            let debut = std::time::Instant::now();
            while !pret(n) && debut.elapsed() < Duration::from_millis(500) {
                thread::sleep(Duration::from_millis(5));
            }
            pret(n)
        };
        assert!(attendre(A_PREPARER), "les voisins du début sont préparés");
        thread::sleep(Duration::from_millis(50)); // il s'endort
        morceau.suivre(40);
        assert!(attendre(40 + A_PREPARER), "réveillé par la platine");
        assert!(pret(40 - A_PREPARER));
    }

    /// On a changé de morceau : le calcul de la forme d'onde de l'ancien
    /// s'arrête au lieu de décoder tout le fichier pour rien.
    #[test]
    fn l_apercu_s_arrete_si_le_morceau_ne_sert_plus() {
        let morceau = Arc::new(Morceau::sinus(440.0, 8000, 5.0));
        let suivi = Arc::downgrade(&morceau);
        assert_eq!(
            Morceau::apercu_tant_qu_utile(&suivi, 100),
            Some(morceau.apercu(100))
        );
        drop(morceau);
        assert_eq!(Morceau::apercu_tant_qu_utile(&suivi, 100), None);
    }
}
