//! Le volume égalisé : tous les morceaux sonnent aussi fort, d'un album à
//! l'autre, sans toucher au curseur de volume.
//!
//! Pour chaque morceau, on connaît son « volume ressenti » :
//! - soit par ses tags ReplayGain, s'il en a (mesurés par un autre logiciel) ;
//! - soit en le mesurant nous-mêmes, une fois, en arrière-plan. La mesure
//!   suit la norme des radios et des sites d'écoute (EBU R128, en LUFS).
//!
//! On en déduit de combien baisser (ou monter) le morceau pour qu'il sonne
//! comme la référence ReplayGain (-18 LUFS). Les mesures sont notées dans un
//! fichier de réglages : chaque morceau n'est mesuré qu'une seule fois.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use eframe::egui;

use id3::frame::ExtendedText;
use id3::TagLike;

use crate::ecriture_tags;
use crate::morceau::Morceau;
use crate::systeme::fichier_de_reglage;
use crate::tags;

/// Le volume visé, en LUFS : celui de la norme ReplayGain 2.0. Les tags
/// ReplayGain et nos propres mesures sont donc comparables.
const REFERENCE: f64 = -18.0;
/// Un morceau très faible n'est jamais monté de plus de 12 dB (×4).
const GAIN_MAX_DB: f32 = 12.0;
/// Le fichier où sont notées les mesures.
const REGLAGE_VOLUMES: &str = "volumes.txt";

/// Ce qu'on sait du volume d'un morceau.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mesure {
    /// De combien (en dB) changer le volume pour atteindre la référence :
    /// négatif pour un morceau trop fort, positif pour un trop faible.
    pub gain_db: f32,
    /// L'échantillon le plus fort du morceau (1.0 = le maximum possible).
    pub crete: f32,
}

impl Mesure {
    /// Le facteur à appliquer au son. Un morceau n'est jamais monté au point
    /// que sa crête dépasse le maximum : il grésillerait (saturation).
    pub fn gain(&self) -> f32 {
        let voulu = 10f32.powf(self.gain_db.min(GAIN_MAX_DB) / 20.0);
        if self.crete > 0.0 {
            voulu.min(1.0 / self.crete).max(voulu.min(1.0))
        } else {
            voulu
        }
    }
}

// ---------------------------------------------------------------------------
// La mesure du volume ressenti (norme UIT BS.1770 / EBU R128).
// ---------------------------------------------------------------------------

/// Un filtre « biquad » : il change le son selon sa hauteur (graves, aigus).
#[derive(Clone)]
struct Filtre {
    b: [f64; 3],
    a: [f64; 2],
    /// Les deux valeurs gardées d'un échantillon à l'autre.
    z: [f64; 2],
}

impl Filtre {
    fn passer(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// La « pondération K » : l'oreille entend mieux les aigus et presque pas
/// les graves très profonds. Deux filtres à la suite, calculés pour la
/// fréquence du morceau (formules de la norme, comme dans libebur128).
fn ponderation_k(frequence: u32) -> [Filtre; 2] {
    let fs = frequence as f64;

    // 1) Les aigus comptent un peu plus (environ +4 dB).
    let f0 = 1681.974450955533;
    let g = 3.999843853973347;
    let q = 0.7071752369554196;
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let aigus = Filtre {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };

    // 2) Les graves très profonds (sous 40 Hz environ) ne comptent presque pas.
    let f0 = 38.13547087602444;
    let q = 0.5003270373238773;
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let graves = Filtre {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    [aigus, graves]
}

/// Mesure le volume ressenti d'un son qu'on lui donne petit à petit.
pub struct Mesureur {
    canaux: usize,
    /// Les filtres de chaque canal.
    filtres: Vec<[Filtre; 2]>,
    /// Le canal du prochain échantillon (le son arrive entrelacé).
    canal: usize,
    /// L'énergie de la tranche de 100 ms en cours, et son nombre de trames.
    energie: f64,
    trames: usize,
    trames_par_tranche: usize,
    /// L'énergie moyenne de chaque tranche de 100 ms déjà finie.
    tranches: Vec<f64>,
    crete: i32,
}

impl Mesureur {
    pub fn new(canaux: usize, frequence: u32) -> Self {
        Self {
            canaux: canaux.max(1),
            filtres: vec![ponderation_k(frequence); canaux.max(1)],
            canal: 0,
            energie: 0.0,
            trames: 0,
            trames_par_tranche: (frequence as usize / 10).max(1),
            tranches: Vec::new(),
            crete: 0,
        }
    }

    /// Ajoute des échantillons (entrelacés : gauche, droite, gauche…).
    pub fn ajouter(&mut self, echantillons: &[i16]) {
        for &e in echantillons {
            self.crete = self.crete.max((e as i32).abs());
            let [aigus, graves] = &mut self.filtres[self.canal];
            let y = graves.passer(aigus.passer(e as f64 / 32768.0));
            self.energie += y * y;
            self.canal += 1;
            if self.canal == self.canaux {
                self.canal = 0;
                self.trames += 1;
                if self.trames == self.trames_par_tranche {
                    self.tranches.push(self.energie / self.trames as f64);
                    self.energie = 0.0;
                    self.trames = 0;
                }
            }
        }
    }

    /// Le résultat. Rien si le morceau est trop court (moins de 0,4 s) ou
    /// entièrement silencieux.
    pub fn resultat(&self) -> Option<Mesure> {
        // Le volume est mesuré par blocs de 400 ms qui se chevauchent
        // (un nouveau toutes les 100 ms), comme le veut la norme.
        let blocs: Vec<f64> = self
            .tranches
            .windows(4)
            .map(|w| w.iter().sum::<f64>() / 4.0)
            .collect();
        let lufs = |energie: f64| -0.691 + 10.0 * energie.log10();
        // On ignore les silences (sous -70 LUFS)…
        let audibles: Vec<f64> = blocs.into_iter().filter(|&e| lufs(e) > -70.0).collect();
        if audibles.is_empty() {
            return None;
        }
        // … puis les passages bien plus calmes que le reste (10 LU sous la moyenne).
        let moyenne = audibles.iter().sum::<f64>() / audibles.len() as f64;
        let seuil = lufs(moyenne) - 10.0;
        let gardes: Vec<f64> = audibles.into_iter().filter(|&e| lufs(e) > seuil).collect();
        let volume = lufs(gardes.iter().sum::<f64>() / gardes.len() as f64);
        Some(Mesure {
            gain_db: (REFERENCE - volume) as f32,
            crete: self.crete as f32 / 32768.0,
        })
    }
}

/// Mesure le volume d'un fichier MP3, en le décodant en entier. Abandonne
/// (et ne renvoie rien) dès que `continuer` renvoie faux.
pub fn mesurer_fichier(chemin: &Path, continuer: impl Fn() -> bool) -> Option<Mesure> {
    let morceau = Morceau::ouvrir_pour_parcourir(chemin, &continuer).ok()?;
    mesurer_morceau(&morceau, continuer)
}

fn mesurer_morceau(morceau: &Morceau, continuer: impl Fn() -> bool) -> Option<Mesure> {
    let mut mesureur = Mesureur::new(morceau.canaux, morceau.frequence);
    let fini = morceau.parcourir(|echantillons| {
        mesureur.ajouter(echantillons);
        continuer()
    });
    if fini {
        mesureur.resultat()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Les volumes connus, mesurés en arrière-plan et notés dans un fichier.
// ---------------------------------------------------------------------------

/// Une mesure notée, avec la taille du fichier à ce moment-là : si le
/// fichier a changé depuis, on le mesure à nouveau.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Notee {
    mesure: Option<Mesure>,
    taille: u64,
}

/// Les volumes des morceaux, mesurés un par un dans un fil d'arrière-plan.
pub struct VolumesConnus {
    partage: Arc<Partage>,
}

struct Partage {
    etat: Mutex<Etat>,
    signal: Condvar,
    /// Mis à vrai quand un morceau urgent (celui qu'on va écouter) est
    /// demandé : la mesure en cours d'un autre morceau s'interrompt.
    urgence: AtomicBool,
    /// Faut-il aussi écrire le volume dans les MP3 (tags ReplayGain) ?
    ecrire: AtomicBool,
}

#[derive(Default)]
struct Etat {
    /// Les mesures notées dans le fichier (pas encore vérifiées).
    notees: HashMap<PathBuf, Notee>,
    /// Les mesures vérifiées (le fichier n'a pas changé) ou toutes fraîches.
    /// `None` : le morceau n'a pas de volume mesurable (silence, illisible).
    connues: HashMap<PathBuf, Option<Mesure>>,
    /// Les fichiers à mesurer, dans l'ordre, et le même ensemble pour ne pas
    /// les demander deux fois.
    a_mesurer: VecDeque<PathBuf>,
    en_attente: HashSet<PathBuf>,
    /// Les fichiers déjà passés au fil pendant que l'écriture dans les MP3
    /// était activée : leur volume y est écrit (ou y était déjà).
    ecrits: HashSet<PathBuf>,
}

impl VolumesConnus {
    /// Reprend les mesures notées et lance le fil de mesure.
    pub fn new(ctx: &egui::Context) -> Self {
        // Pendant les tests, on ne touche pas aux vraies mesures.
        let fichier = if cfg!(test) {
            None
        } else {
            fichier_de_reglage(REGLAGE_VOLUMES)
        };
        let notees = fichier
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|texte| lire_notes(&texte))
            .unwrap_or_default();
        let partage = Arc::new(Partage {
            etat: Mutex::new(Etat {
                notees,
                ..Default::default()
            }),
            signal: Condvar::new(),
            urgence: AtomicBool::new(false),
            ecrire: AtomicBool::new(false),
        });
        let pour_le_fil = Arc::clone(&partage);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            mesurer_en_arriere_plan(&pour_le_fil, fichier.as_deref(), &|| ctx.request_repaint())
        });
        Self { partage }
    }

    fn etat(&self) -> std::sync::MutexGuard<'_, Etat> {
        self.partage.etat.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Demande les volumes de ces morceaux. Le premier est urgent (c'est
    /// celui qu'on écoute) : il passe devant tous les autres.
    pub fn demander<'a>(&self, chemins: impl IntoIterator<Item = &'a Path>) {
        let mut etat = self.etat();
        let ecrire = self.partage.ecrire.load(Ordering::Relaxed);
        for (n, chemin) in chemins.into_iter().enumerate() {
            // Déjà connu (et déjà écrit dans le fichier, si on doit l'y écrire).
            if etat.connues.contains_key(chemin) && (!ecrire || etat.ecrits.contains(chemin)) {
                continue;
            }
            if n == 0 {
                if etat.a_mesurer.front().map(PathBuf::as_path) != Some(chemin) {
                    etat.a_mesurer.retain(|c| c != chemin);
                    etat.a_mesurer.push_front(chemin.to_path_buf());
                    etat.en_attente.insert(chemin.to_path_buf());
                    self.partage.urgence.store(true, Ordering::Relaxed);
                }
            } else if etat.en_attente.insert(chemin.to_path_buf()) {
                etat.a_mesurer.push_back(chemin.to_path_buf());
            }
        }
        self.partage.signal.notify_one();
    }

    /// Active ou non l'écriture du volume dans les MP3 (tags ReplayGain) des
    /// morceaux demandés ensuite, déjà mesurés ou non.
    pub fn ecrire_dans_les_fichiers(&self, ecrire: bool) {
        self.partage.ecrire.store(ecrire, Ordering::Relaxed);
    }

    /// Le facteur de volume de ce morceau, s'il est déjà connu. 1.0 pour un
    /// morceau sans volume mesurable (on n'y touche pas).
    pub fn gain(&self, chemin: &Path) -> Option<f32> {
        let etat = self.etat();
        let mesure = etat.connues.get(chemin)?;
        Some(mesure.map_or(1.0, |m| m.gain()))
    }
}

#[cfg(test)]
impl VolumesConnus {
    /// Pour les tests : fait comme si ce morceau avait déjà été mesuré.
    pub fn fixer(&self, chemin: &Path, mesure: Mesure) {
        self.etat()
            .connues
            .insert(chemin.to_path_buf(), Some(mesure));
    }
}

/// La boucle du fil de mesure : prend le prochain morceau, le mesure (sans
/// tenir le verrou), écrit le volume dans le fichier si c'est demandé, et
/// range le résultat.
fn mesurer_en_arriere_plan(partage: &Partage, fichier: Option<&Path>, prevenir: &dyn Fn()) {
    loop {
        let (chemin, connue, notee) = {
            let mut etat = partage.etat.lock().unwrap_or_else(|e| e.into_inner());
            while etat.a_mesurer.is_empty() {
                etat = partage.signal.wait(etat).unwrap_or_else(|e| e.into_inner());
            }
            let chemin = etat.a_mesurer.pop_front().unwrap();
            partage.urgence.store(false, Ordering::Relaxed);
            let connue = etat.connues.get(&chemin).copied();
            let notee = etat.notees.get(&chemin).copied();
            (chemin, connue, notee)
        };
        let mut taille = std::fs::metadata(&chemin).map_or(0, |m| m.len());
        let resultat = match (connue, notee) {
            // Déjà mesuré pendant cette séance (on vient juste l'écrire).
            (Some(mesure), _) => Some(mesure),
            // Déjà mesuré, et le fichier n'a pas changé.
            (None, Some(notee)) if notee.taille == taille => Some(notee.mesure),
            _ => {
                let continuer = || !partage.urgence.load(Ordering::Relaxed);
                match tags::lire_replaygain(&chemin) {
                    Some(mesure) => Some(Some(mesure)),
                    None => match mesurer_fichier(&chemin, continuer) {
                        Some(mesure) => Some(Some(mesure)),
                        // Interrompu par un morceau urgent : on le remet en file.
                        None if !continuer() => None,
                        None => Some(None),
                    },
                }
                .inspect(|&mesure| {
                    if let Some(fichier) = fichier {
                        noter(fichier, &chemin, Notee { mesure, taille });
                    }
                })
            }
        };
        // Écriture dans le MP3, s'il n'a pas déjà ses tags ReplayGain. Un
        // échec (lecture seule, fichier ouvert ailleurs) n'empêche pas
        // l'égalisation : on réessaiera la prochaine fois.
        let ecrire = partage.ecrire.load(Ordering::Relaxed);
        let mut ecrit = false;
        if let (true, Some(Some(mesure))) = (ecrire, resultat) {
            ecrit = tags::lire_replaygain(&chemin).is_some()
                || ecrire_replaygain(&chemin, mesure)
                    .inspect_err(|e| {
                        eprintln!("[volume] écriture impossible ({e}) : {}", chemin.display())
                    })
                    .is_ok_and(|()| {
                        // Le fichier a grandi : on note sa nouvelle taille.
                        taille = std::fs::metadata(&chemin).map_or(0, |m| m.len());
                        if let Some(fichier) = fichier {
                            noter(
                                fichier,
                                &chemin,
                                Notee {
                                    mesure: Some(mesure),
                                    taille,
                                },
                            );
                        }
                        true
                    });
        }
        let mut etat = partage.etat.lock().unwrap_or_else(|e| e.into_inner());
        match resultat {
            Some(mesure) => {
                etat.en_attente.remove(&chemin);
                if ecrit {
                    etat.ecrits.insert(chemin.clone());
                }
                if ecrit && taille != 0 {
                    etat.notees.insert(chemin.clone(), Notee { mesure, taille });
                }
                etat.connues.insert(chemin, mesure);
                prevenir();
            }
            None => etat.a_mesurer.push_back(chemin),
        }
    }
}

/// Écrit le volume mesuré dans les tags ReplayGain du fichier (« par
/// morceau »), comme le font foobar2000 ou MusicBee. Les autres tags ne
/// changent pas, et la musique est vérifiée avant de remplacer le fichier.
pub fn ecrire_replaygain(chemin: &Path, mesure: Mesure) -> Result<(), ecriture_tags::Erreur> {
    ecriture_tags::modifier(chemin, |tag| {
        tag.add_frame(ExtendedText {
            description: "REPLAYGAIN_TRACK_GAIN".to_string(),
            value: format!("{:+.2} dB", mesure.gain_db),
        });
        tag.add_frame(ExtendedText {
            description: "REPLAYGAIN_TRACK_PEAK".to_string(),
            value: format!("{:.6}", mesure.crete),
        });
    })
}

/// Lit le fichier des mesures : une ligne par morceau,
/// « gain_db <tab> crête <tab> taille <tab> chemin ». « - » à la place du gain
/// pour un morceau sans volume mesurable. En cas de doublon, la dernière
/// ligne l'emporte (on ajoute toujours à la fin du fichier).
fn lire_notes(texte: &str) -> HashMap<PathBuf, Notee> {
    texte
        .lines()
        .filter_map(|ligne| {
            let mut champs = ligne.splitn(4, '\t');
            let gain = champs.next()?;
            let crete: f32 = champs.next()?.parse().ok()?;
            let taille: u64 = champs.next()?.parse().ok()?;
            let chemin = champs.next().filter(|c| !c.is_empty())?;
            let mesure = match gain {
                "-" => None,
                gain => Some(Mesure {
                    gain_db: gain.parse().ok()?,
                    crete,
                }),
            };
            Some((PathBuf::from(chemin), Notee { mesure, taille }))
        })
        .collect()
}

/// Une ligne du fichier des mesures (voir `lire_notes`).
fn ligne_de_note(chemin: &Path, notee: Notee) -> String {
    let (gain, crete) = match notee.mesure {
        Some(m) => (m.gain_db.to_string(), m.crete),
        None => ("-".to_string(), 0.0),
    };
    // Les nombres sont écrits en entier, pour être relus à l'identique.
    format!("{gain}\t{crete}\t{}\t{}\n", notee.taille, chemin.display())
}

/// Ajoute une mesure à la fin du fichier. Si on n'y arrive pas, tant pis :
/// le morceau sera simplement mesuré à nouveau au prochain lancement.
fn noter(fichier: &Path, chemin: &Path, notee: Notee) {
    if let Some(dossier) = fichier.parent() {
        let _ = std::fs::create_dir_all(dossier);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(fichier)
    {
        let _ = f.write_all(ligne_de_note(chemin, notee).as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un son pur (sinusoïde) de `hz` Hz, d'amplitude `amplitude` (1.0 = le
    /// maximum), sur `canaux` canaux identiques.
    fn sinus(hz: f64, amplitude: f64, frequence: u32, secondes: f64, canaux: usize) -> Vec<i16> {
        let n = (frequence as f64 * secondes) as usize;
        (0..n)
            .flat_map(|i| {
                let t = i as f64 / frequence as f64;
                let e = ((std::f64::consts::TAU * hz * t).sin() * amplitude * 32767.0) as i16;
                std::iter::repeat_n(e, canaux)
            })
            .collect()
    }

    fn mesurer(echantillons: &[i16], canaux: usize, frequence: u32) -> Option<Mesure> {
        let mut mesureur = Mesureur::new(canaux, frequence);
        mesureur.ajouter(echantillons);
        mesureur.resultat()
    }

    /// Le volume en LUFS, retrouvé à partir du gain.
    fn lufs(mesure: Mesure) -> f64 {
        REFERENCE - mesure.gain_db as f64
    }

    /// La norme le dit : un sinus de 1 kHz au maximum sur un canal mesure -3,01 LUFS.
    #[test]
    fn un_sinus_de_1_khz_au_maximum_donne_moins_3_lufs() {
        for frequence in [44_100, 48_000] {
            let mesure = mesurer(&sinus(1000.0, 1.0, frequence, 5.0, 1), 1, frequence).unwrap();
            let volume = lufs(mesure);
            assert!(
                (volume + 3.01).abs() < 0.05,
                "{frequence} Hz : {volume} LUFS"
            );
            assert!((mesure.crete - 1.0).abs() < 0.001);
        }
    }

    /// Deux fois moins fort (-6 dB) : 6 LUFS de moins. En stéréo, deux
    /// canaux identiques comptent double : +3 LUFS.
    #[test]
    fn moitie_moins_fort_et_stereo() {
        let moitie = mesurer(&sinus(1000.0, 0.5, 48_000, 5.0, 1), 1, 48_000).unwrap();
        assert!((lufs(moitie) + 9.03).abs() < 0.05, "{}", lufs(moitie));
        let stereo = mesurer(&sinus(1000.0, 0.5, 48_000, 5.0, 2), 2, 48_000).unwrap();
        assert!((lufs(stereo) + 6.02).abs() < 0.05, "{}", lufs(stereo));
    }

    /// Les graves très profonds comptent beaucoup moins que les médiums.
    #[test]
    fn les_graves_profonds_comptent_moins() {
        let grave = mesurer(&sinus(20.0, 1.0, 48_000, 5.0, 1), 1, 48_000).unwrap();
        assert!(lufs(grave) < -10.0, "{}", lufs(grave));
    }

    /// Les silences ne font (presque) pas baisser la mesure : un morceau
    /// avec une longue fin silencieuse sonne aussi fort que sans. Seuls les
    /// blocs à cheval sur la fin du son comptent un peu.
    #[test]
    fn les_silences_ne_comptent_pas() {
        let mut son = sinus(1000.0, 0.5, 48_000, 5.0, 1);
        let seul = lufs(mesurer(&son, 1, 48_000).unwrap());
        son.extend(std::iter::repeat_n(0, 48_000 * 20));
        let avec_silence = lufs(mesurer(&son, 1, 48_000).unwrap());
        assert!((seul - avec_silence).abs() < 0.2, "{seul} / {avec_silence}");
    }

    #[test]
    fn un_silence_n_a_pas_de_volume() {
        assert_eq!(mesurer(&vec![0; 48_000 * 5], 1, 48_000), None);
        assert_eq!(
            mesurer(&sinus(1000.0, 1.0, 48_000, 0.2, 1), 1, 48_000),
            None
        );
    }

    /// Envoyer le son d'un coup ou petit à petit donne la même mesure.
    #[test]
    fn la_mesure_ne_depend_pas_du_decoupage() {
        let son = sinus(440.0, 0.3, 44_100, 3.0, 2);
        let d_un_coup = mesurer(&son, 2, 44_100);
        let mut mesureur = Mesureur::new(2, 44_100);
        for morceau in son.chunks(1151) {
            mesureur.ajouter(morceau);
        }
        assert_eq!(mesureur.resultat(), d_un_coup);
    }

    #[test]
    fn un_morceau_trop_fort_est_baisse() {
        let mesure = Mesure {
            gain_db: -6.0,
            crete: 1.0,
        };
        assert!((mesure.gain() - 0.501).abs() < 0.001);
    }

    /// Un morceau faible est monté, mais jamais au point de saturer, et
    /// jamais de plus de 12 dB.
    #[test]
    fn un_morceau_faible_est_monte_sans_saturer() {
        let libre = Mesure {
            gain_db: 6.0,
            crete: 0.25,
        };
        assert!((libre.gain() - 1.995).abs() < 0.001);
        let limite = Mesure {
            gain_db: 6.0,
            crete: 0.8,
        };
        assert_eq!(limite.gain(), 1.25);
        let au_maximum = Mesure {
            gain_db: 6.0,
            crete: 1.0,
        };
        assert_eq!(au_maximum.gain(), 1.0);
        let tres_faible = Mesure {
            gain_db: 30.0,
            crete: 0.01,
        };
        assert!((tres_faible.gain() - 3.981).abs() < 0.001);
    }

    /// Le vrai MP3 de test (un glissando) se mesure, et un abandon ne donne rien.
    #[test]
    fn mesure_d_un_vrai_mp3() {
        let chemin = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glissando.mp3");
        let mesure = mesurer_fichier(&chemin, || true).expect("mesure");
        assert!((-40.0..0.0).contains(&lufs(mesure)), "{}", lufs(mesure));
        assert!(mesure.crete > 0.0 && mesure.crete <= 1.0);
        assert_eq!(mesurer_fichier(&chemin, || false), None);
    }

    #[test]
    fn les_notes_se_relisent() {
        let mesure = Notee {
            mesure: Some(Mesure {
                gain_db: -7.25,
                crete: 0.9876,
            }),
            taille: 4_000_123,
        };
        let silence = Notee {
            mesure: None,
            taille: 12,
        };
        let a = Path::new("C:\\Musique\\Zebda\\Tomber la chemise.mp3");
        let b = Path::new("/home/luce/Musique/silence.mp3");
        let texte = ligne_de_note(a, mesure) + &ligne_de_note(b, silence) + "ligne abîmée\n";
        let notes = lire_notes(&texte);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[a], mesure);
        assert_eq!(notes[b], silence);
        // Une nouvelle mesure ajoutée à la fin remplace l'ancienne.
        let nouvelle = Notee {
            mesure: None,
            taille: 5,
        };
        let notes = lire_notes(&(texte + &ligne_de_note(a, nouvelle)));
        assert_eq!(notes[a], nouvelle);
    }

    /// Un dossier de test vide avec une copie du vrai MP3 de test (sans
    /// tags ReplayGain), et le chemin du fichier des mesures.
    fn dossier_de_test(nom: &str) -> (PathBuf, PathBuf) {
        let dossier = std::env::temp_dir().join(format!("lecteur_mp3_test_{nom}"));
        let _ = std::fs::remove_dir_all(&dossier);
        std::fs::create_dir_all(&dossier).unwrap();
        let morceau = dossier.join("glissando.mp3");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glissando.mp3"),
            &morceau,
        )
        .unwrap();
        (morceau, dossier.join("volumes.txt"))
    }

    /// Lance un fil de mesure qui note dans `notes`. Le récepteur reçoit un
    /// signal à chaque morceau traité.
    fn lancer(
        notees: HashMap<PathBuf, Notee>,
        notes: &Path,
        ecrire: bool,
    ) -> (VolumesConnus, std::sync::mpsc::Receiver<()>) {
        let partage = Arc::new(Partage {
            etat: Mutex::new(Etat {
                notees,
                ..Default::default()
            }),
            signal: Condvar::new(),
            urgence: AtomicBool::new(false),
            ecrire: AtomicBool::new(ecrire),
        });
        let (pour_le_fil, notes) = (Arc::clone(&partage), notes.to_path_buf());
        let (envoi, reception) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            mesurer_en_arriere_plan(&pour_le_fil, Some(&notes), &|| {
                let _ = envoi.send(());
            })
        });
        (VolumesConnus { partage }, reception)
    }

    fn attendre(fini: &std::sync::mpsc::Receiver<()>) {
        fini.recv_timeout(std::time::Duration::from_secs(30))
            .expect("le fil de mesure ne répond pas");
    }

    fn relire_notes(notes: &Path) -> HashMap<PathBuf, Notee> {
        lire_notes(&std::fs::read_to_string(notes).unwrap_or_default())
    }

    /// Le fil de mesure : le fichier est mesuré, noté, puis retrouvé dans
    /// les notes sans être mesuré à nouveau. Sans l'option, le MP3 n'est
    /// pas touché.
    #[test]
    fn les_volumes_sont_mesures_puis_notes() {
        let (morceau, notes) = dossier_de_test("volumes");
        let original = std::fs::read(&morceau).unwrap();

        let (volumes, fini) = lancer(HashMap::new(), &notes, false);
        assert_eq!(volumes.gain(&morceau), None);
        volumes.demander([morceau.as_path()]);
        attendre(&fini);
        let gain = volumes.gain(&morceau).expect("mesuré");
        let notees = relire_notes(&notes);
        assert_eq!(notees.len(), 1);
        assert_eq!(std::fs::read(&morceau).unwrap(), original);

        // Au lancement suivant, la note suffit : le fichier de notes ne grandit pas.
        let (volumes, fini) = lancer(notees, &notes, false);
        volumes.demander([morceau.as_path()]);
        attendre(&fini);
        assert_eq!(volumes.gain(&morceau), Some(gain));
        assert_eq!(std::fs::read_to_string(&notes).unwrap().lines().count(), 1);
    }

    /// Les tags écrits se relisent, la musique ne change pas, et écrire
    /// deux fois ne crée pas de tags en double.
    #[test]
    fn le_volume_s_ecrit_dans_le_mp3() {
        let (morceau, _) = dossier_de_test("ecrire_replaygain");
        let musique = |chemin: &Path| {
            let mut sans_tags = Vec::new();
            Morceau::ouvrir_pour_parcourir(chemin, || true)
                .unwrap()
                .parcourir(|e| {
                    sans_tags.extend_from_slice(e);
                    true
                });
            sans_tags
        };
        let avant = musique(&morceau);
        let mesure = Mesure {
            gain_db: -6.537,
            crete: 0.65078735,
        };
        ecrire_replaygain(&morceau, mesure).unwrap();
        ecrire_replaygain(&morceau, mesure).unwrap();
        assert_eq!(
            tags::lire_replaygain(&morceau),
            Some(Mesure {
                gain_db: -6.54,
                crete: 0.650787
            })
        );
        assert_eq!(musique(&morceau), avant);
        let tag = id3::Tag::read_from_path(&morceau).unwrap();
        assert_eq!(tag.extended_texts().count(), 2);
        let gain = tag
            .extended_texts()
            .find(|t| t.description == "REPLAYGAIN_TRACK_GAIN");
        assert_eq!(gain.unwrap().value, "-6.54 dB");
    }

    /// Avec l'option : le volume mesuré est écrit dans le MP3, et la note
    /// suit la nouvelle taille du fichier (pas de nouvelle mesure ensuite).
    #[test]
    fn avec_l_option_le_volume_mesure_est_ecrit() {
        let (morceau, notes) = dossier_de_test("volumes_ecrits");
        let (volumes, fini) = lancer(HashMap::new(), &notes, true);
        volumes.demander([morceau.as_path()]);
        attendre(&fini);
        let gain = volumes.gain(&morceau).expect("mesuré");
        let ecrit = tags::lire_replaygain(&morceau).expect("tags écrits");
        assert!((ecrit.gain() - gain).abs() < 0.01);
        let taille = std::fs::metadata(&morceau).unwrap().len();
        assert_eq!(relire_notes(&notes)[&morceau].taille, taille);

        // Au lancement suivant : rien à mesurer ni à écrire.
        let (volumes, fini) = lancer(relire_notes(&notes), &notes, true);
        volumes.demander([morceau.as_path()]);
        attendre(&fini);
        assert_eq!(volumes.gain(&morceau), Some(gain));
        assert_eq!(std::fs::metadata(&morceau).unwrap().len(), taille);
    }

    /// Cocher l'option après coup : un morceau déjà connu est écrit à sa
    /// prochaine demande. Des tags ReplayGain déjà là ne sont jamais changés.
    #[test]
    fn l_option_cochee_apres_coup_et_tags_existants() {
        let (morceau, notes) = dossier_de_test("volumes_apres_coup");
        let (volumes, fini) = lancer(HashMap::new(), &notes, false);
        volumes.demander([morceau.as_path()]);
        attendre(&fini);
        assert_eq!(tags::lire_replaygain(&morceau), None);
        volumes.ecrire_dans_les_fichiers(true);
        volumes.demander([morceau.as_path()]);
        attendre(&fini);
        assert!(tags::lire_replaygain(&morceau).is_some());

        // Un autre fichier qui a déjà ses propres tags ReplayGain.
        let (autre, notes) = dossier_de_test("volumes_tags_existants");
        let siens = Mesure {
            gain_db: -1.0,
            crete: 0.5,
        };
        ecrire_replaygain(&autre, siens).unwrap();
        let (volumes, fini) = lancer(HashMap::new(), &notes, true);
        volumes.demander([autre.as_path()]);
        attendre(&fini);
        assert_eq!(tags::lire_replaygain(&autre), Some(siens));
        assert_eq!(volumes.gain(&autre), Some(siens.gain()));
    }
}
