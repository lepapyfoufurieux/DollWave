//! « Infos du fichier » : la qualité d'un MP3 (débit, fréquence, mono ou
//! stéréo), sa taille et son emplacement sur le disque. On l'ouvre d'un clic
//! droit sur un morceau, ou d'un clic sur la petite ligne sous la pochette du
//! morceau en cours.
//!
//! Pour le débit, on parcourt toutes les « trames » du MP3 (des petits
//! morceaux d'environ 26 ms, chacun avec son en-tête de 4 octets) : si elles
//! ont toutes le même débit, il est constant (CBR) ; sinon il est variable
//! (VBR) et on affiche sa moyenne. C'est exact, et rapide : on ne lit que les
//! en-têtes, sans décoder la musique.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

use crate::liste::nom_du_fichier;
use crate::systeme;
use crate::tags::TagsConnus;

/// Le débit du MP3, en kilobits par seconde.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Debit {
    /// Le même pour tout le morceau (CBR).
    Constant(u32),
    /// Il change selon les passages (VBR) : sa moyenne.
    Variable(u32),
}

/// Comment les deux côtés du son sont rangés.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Canaux {
    Stereo,
    /// Stéréo où la gauche et la droite sont codées ensemble (le plus
    /// courant : ça économise de la place, sans rien perdre de la stéréo).
    StereoJointe,
    /// Deux pistes mono indépendantes (rare).
    DoubleMono,
    Mono,
}

/// La qualité d'un MP3, lue dans ses en-têtes de trames.
#[derive(Clone, Debug, PartialEq)]
pub struct Qualite {
    pub debit: Debit,
    /// La fréquence d'échantillonnage, en hertz (44 100 pour un CD).
    pub frequence: u32,
    pub canaux: Canaux,
    /// « MPEG-1 », « MPEG-2 » ou « MPEG-2.5 ».
    pub version: &'static str,
    pub duree: Duration,
}

/// Ce qu'on sait d'un fichier.
#[derive(Clone, Debug, PartialEq)]
pub struct Infos {
    /// La taille, en octets.
    pub taille: u64,
    /// Rien si le fichier n'a pas de trame MP3 reconnaissable.
    pub qualite: Option<Qualite>,
}

/// Lit les infos d'un fichier (à faire en arrière-plan : il faut le lire en
/// entier). Le fichier tient en mémoire le temps de l'analyse, comme le
/// morceau en cours pour le scratch.
pub fn lire(chemin: &Path) -> Result<Infos, String> {
    let octets = std::fs::read(chemin).map_err(|e| e.to_string())?;
    Ok(Infos {
        taille: octets.len() as u64,
        qualite: analyser(&octets),
    })
}

// --- Les en-têtes de trames MP3 ---

/// Débits possibles en kbit/s (MP3 = « Layer III »), selon le numéro rangé
/// dans l'en-tête. 0 = « format libre », qu'on ne gère pas.
const DEBITS_MPEG1: [u32; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
const DEBITS_MPEG2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
const FREQUENCES_MPEG1: [u32; 3] = [44_100, 48_000, 32_000];

/// L'en-tête d'une trame : ses 4 premiers octets.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entete {
    /// 1 = MPEG-1, 2 = MPEG-2, 25 = MPEG-2.5.
    version: u8,
    debit: u32,
    frequence: u32,
    canaux: Canaux,
    /// La longueur de la trame, en-tête compris, en octets.
    longueur: usize,
}

impl Entete {
    /// Lit l'en-tête qui commence à ces octets, s'il est valide.
    fn lire(o: &[u8]) -> Option<Self> {
        let [a, b, c, d] = *o.get(..4)? else {
            return None;
        };
        // 11 bits à 1 : le « mot de synchronisation » d'une trame.
        if a != 0xFF || b & 0xE0 != 0xE0 {
            return None;
        }
        let version = match (b >> 3) & 3 {
            0 => 25,
            2 => 2,
            3 => 1,
            _ => return None,
        };
        // Layer III seulement (01).
        if (b >> 1) & 3 != 1 {
            return None;
        }
        let numero_debit = (c >> 4) as usize;
        let numero_frequence = ((c >> 2) & 3) as usize;
        if numero_debit == 0 || numero_debit == 15 || numero_frequence == 3 {
            return None;
        }
        let debit = if version == 1 {
            DEBITS_MPEG1[numero_debit]
        } else {
            DEBITS_MPEG2[numero_debit]
        };
        let frequence = FREQUENCES_MPEG1[numero_frequence]
            / match version {
                1 => 1,
                2 => 2,
                _ => 4,
            };
        let remplissage = ((c >> 1) & 1) as usize;
        let canaux = match d >> 6 {
            0 => Canaux::Stereo,
            1 => Canaux::StereoJointe,
            2 => Canaux::DoubleMono,
            _ => Canaux::Mono,
        };
        // 1152 échantillons par trame en MPEG-1, 576 sinon.
        let facteur = if version == 1 { 144 } else { 72 };
        let longueur = (facteur * debit * 1000 / frequence) as usize + remplissage;
        Some(Self {
            version,
            debit,
            frequence,
            canaux,
            longueur,
        })
    }

    fn echantillons(&self) -> u32 {
        if self.version == 1 {
            1152
        } else {
            576
        }
    }

    /// Une autre trame du même morceau ? (Même version, même fréquence.)
    fn meme_flux(&self, autre: &Self) -> bool {
        self.version == autre.version && self.frequence == autre.frequence
    }

    /// Cette trame est-elle l'en-tête « Xing », « Info » ou « VBRI » que les
    /// encodeurs placent au début du fichier ? Elle ne contient pas de son :
    /// on ne la compte pas.
    fn est_un_resume(&self, trame: &[u8]) -> bool {
        let mono = self.canaux == Canaux::Mono;
        let decalage = 4 + match (self.version == 1, mono) {
            (true, false) => 32,
            (true, true) | (false, false) => 17,
            (false, true) => 9,
        };
        let marque = |depuis: usize| trame.get(depuis..depuis + 4);
        matches!(marque(decalage), Some(b"Xing" | b"Info")) || marque(36) == Some(b"VBRI")
    }
}

/// La taille des tags ID3v2 au début du fichier (0 s'il n'y en a pas). Il
/// peut y en avoir plusieurs à la suite.
fn taille_des_tags_id3(octets: &[u8]) -> usize {
    let mut debut = 0;
    while let Some(entete) = octets.get(debut..debut + 10) {
        if &entete[..3] != b"ID3" {
            break;
        }
        // La taille est sur 4 octets de 7 bits.
        let taille = entete[6..10]
            .iter()
            .fold(0usize, |t, &o| (t << 7) | (o & 0x7F) as usize);
        let pied = if entete[5] & 0x10 != 0 { 10 } else { 0 };
        debut += 10 + taille + pied;
    }
    debut
}

/// La première vraie trame à partir de `depuis` : un en-tête valide suivi
/// d'un autre en-tête valide du même flux (pour ne pas se laisser tromper
/// par des octets qui ressemblent à un en-tête par hasard).
fn trouver_une_trame(octets: &[u8], depuis: usize) -> Option<(usize, Entete)> {
    (depuis..octets.len().saturating_sub(3)).find_map(|i| {
        let entete = Entete::lire(&octets[i..])?;
        let fin = i + entete.longueur;
        // Une trame seule tout au bout du fichier : on l'accepte.
        let suivie = fin == octets.len()
            || Entete::lire(octets.get(fin..)?).is_some_and(|s| s.meme_flux(&entete));
        suivie.then_some((i, entete))
    })
}

/// Parcourt les trames du MP3. Rien si on n'en trouve aucune.
fn analyser(octets: &[u8]) -> Option<Qualite> {
    let (mut position, premiere) = trouver_une_trame(octets, taille_des_tags_id3(octets))?;
    let (mut trames, mut octets_son) = (0u64, 0u64);
    let (mut debit_min, mut debit_max) = (u32::MAX, 0);
    let mut stereo_jointe = false;
    let mut entete = premiere;
    let mut premiere_trame = true;
    loop {
        let fin = (position + entete.longueur).min(octets.len());
        let resume = premiere_trame && entete.est_un_resume(&octets[position..fin]);
        premiere_trame = false;
        if !resume {
            trames += 1;
            octets_son += (fin - position) as u64;
            debit_min = debit_min.min(entete.debit);
            debit_max = debit_max.max(entete.debit);
            stereo_jointe |= entete.canaux == Canaux::StereoJointe;
        }
        position = fin;
        // La trame suivante, juste après ; sinon (un petit trou, ou la fin du
        // fichier avec ses tags ID3v1/APE), on la cherche plus loin.
        match Entete::lire(octets.get(position..).unwrap_or_default())
            .filter(|e| e.meme_flux(&premiere))
            .map(|e| (position, e))
            .or_else(|| trouver_une_trame(octets, position).filter(|(_, e)| e.meme_flux(&premiere)))
        {
            Some((suivante, e)) => {
                position = suivante;
                entete = e;
            }
            None => break,
        }
    }
    if trames == 0 {
        return None;
    }
    let secondes =
        (trames * u64::from(premiere.echantillons())) as f64 / f64::from(premiere.frequence);
    let debit = if debit_min == debit_max {
        Debit::Constant(debit_min)
    } else {
        Debit::Variable((octets_son as f64 * 8.0 / secondes / 1000.0).round() as u32)
    };
    // Un MP3 « stéréo jointe » peut avoir des trames notées simplement
    // stéréo : s'il y en a une seule jointe, on le dit stéréo jointe.
    let canaux = if stereo_jointe {
        Canaux::StereoJointe
    } else {
        premiere.canaux
    };
    Some(Qualite {
        debit,
        frequence: premiere.frequence,
        canaux,
        version: match premiere.version {
            1 => "MPEG-1",
            2 => "MPEG-2",
            _ => "MPEG-2.5",
        },
        duree: Duration::from_secs_f64(secondes),
    })
}

// --- Les textes à afficher ---

impl Debit {
    pub fn texte(&self) -> String {
        match self {
            Debit::Constant(k) => format!("{k} kbit/s"),
            Debit::Variable(k) => format!("~{k} kbit/s"),
        }
    }

    fn texte_complet(&self) -> String {
        match self {
            Debit::Constant(_) => format!("{} (constant, CBR)", self.texte()),
            Debit::Variable(_) => format!("{} en moyenne (variable, VBR)", self.texte()),
        }
    }
}

impl Canaux {
    fn texte(&self) -> &'static str {
        match self {
            Canaux::Stereo => "Stéréo",
            Canaux::StereoJointe => "Stéréo (jointe)",
            Canaux::DoubleMono => "Double mono",
            Canaux::Mono => "Mono",
        }
    }
}

/// « 44,1 kHz », « 48 kHz ».
fn texte_frequence(hertz: u32) -> String {
    let texte = format!("{:.1}", f64::from(hertz) / 1000.0);
    let texte = texte.strip_suffix(".0").unwrap_or(&texte).replace('.', ",");
    format!("{texte} kHz")
}

/// « 8,4 Mo », « 512 Ko », comme l'Explorateur Windows (1 Mo = 1024 Ko).
pub fn texte_taille(octets: u64) -> String {
    const KO: f64 = 1024.0;
    let octets_f = octets as f64;
    let texte = if octets_f >= KO * KO * KO {
        format!("{:.1} Go", octets_f / (KO * KO * KO))
    } else if octets_f >= KO * KO {
        format!("{:.1} Mo", octets_f / (KO * KO))
    } else if octets_f >= KO {
        format!("{:.0} Ko", octets_f / KO)
    } else {
        format!("{octets} octets")
    };
    texte.replace('.', ",")
}

/// « 8 812 345 » : les milliers séparés par une espace.
fn avec_espaces(nombre: u64) -> String {
    let chiffres = nombre.to_string();
    let mut texte = String::new();
    for (i, c) in chiffres.chars().enumerate() {
        if i > 0 && (chiffres.len() - i).is_multiple_of(3) {
            texte.push('\u{202F}');
        }
        texte.push(c);
    }
    texte
}

/// La petite ligne sous la pochette : « MP3 · 320 kbit/s · 44,1 kHz ·
/// stéréo · 8,4 Mo ».
pub fn resume(infos: &Infos) -> String {
    let mut morceaux = vec!["MP3".to_string()];
    if let Some(q) = &infos.qualite {
        morceaux.push(q.debit.texte());
        morceaux.push(texte_frequence(q.frequence));
        morceaux.push(
            match q.canaux {
                Canaux::Mono => "mono",
                _ => "stéréo",
            }
            .to_string(),
        );
    }
    morceaux.push(texte_taille(infos.taille));
    morceaux.join(" · ")
}

// --- Les infos du morceau en cours, lues en arrière-plan ---

/// Les infos déjà lues (ou en cours de lecture) de quelques fichiers.
#[derive(Default)]
pub struct InfosConnues {
    connues: Arc<Mutex<HashMap<PathBuf, Option<Infos>>>>,
}

impl InfosConnues {
    /// Les infos de ce fichier, si elles sont prêtes. Sinon, on les lit en
    /// arrière-plan et la fenêtre sera redessinée quand ce sera fait.
    pub fn demander(&self, ctx: &egui::Context, chemin: &Path) -> Option<Infos> {
        let mut connues = self.connues.lock().ok()?;
        if let Some(infos) = connues.get(chemin) {
            return infos.clone();
        }
        connues.insert(chemin.to_path_buf(), None);
        let (connues, chemin, ctx) = (self.connues.clone(), chemin.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let infos = lire(&chemin).ok();
            if let (Some(infos), Ok(mut connues)) = (infos, connues.lock()) {
                connues.insert(chemin, Some(infos));
            }
            ctx.request_repaint();
        });
        None
    }

    /// Le fichier a changé (tags réécrits) : on relira ses infos.
    pub fn oublier(&self, chemin: &Path) {
        if let Ok(mut connues) = self.connues.lock() {
            connues.remove(chemin);
        }
    }
}

// --- La fenêtre ---

pub struct FenetreInfos {
    chemin: PathBuf,
    nom: String,
    infos: Option<Result<Infos, String>>,
    reception: Option<Receiver<Result<Infos, String>>>,
    /// « Chemin copié », pendant un moment après le clic.
    copie: bool,
    /// Le message d'erreur si l'Explorateur n'a pas pu s'ouvrir.
    erreur: Option<String>,
}

impl FenetreInfos {
    /// Ouvre la fenêtre et lit le fichier en arrière-plan (relu à chaque
    /// ouverture, pour être à jour).
    pub fn ouvrir(ctx: &egui::Context, chemin: PathBuf, tags: &TagsConnus) -> Self {
        let (envoi, reception) = mpsc::channel();
        let (fichier, ctx) = (chemin.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = envoi.send(lire(&fichier));
            ctx.request_repaint();
        });
        Self {
            nom: tags.libelle(&chemin),
            chemin,
            infos: None,
            reception: Some(reception),
            copie: false,
            erreur: None,
        }
    }

    /// Dessine la fenêtre. Rend `false` quand elle est à fermer.
    pub fn afficher(&mut self, ctx: &egui::Context) -> bool {
        if let Some(reception) = &self.reception {
            match reception.try_recv() {
                Ok(infos) => {
                    self.infos = Some(infos);
                    self.reception = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.reception = None,
            }
        }
        let mut ouverte = true;
        let mut fermer = false;
        egui::Window::new("ℹ Infos du fichier")
            .id(egui::Id::new("infos_fichier"))
            .open(&mut ouverte)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.strong(&self.nom);
                ui.add_space(6.0);
                egui::Grid::new("infos_fichier_grille")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| self.grille(ui));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("📂 Ouvrir l'emplacement")
                        .on_hover_text("Ouvre le dossier avec ce fichier sélectionné")
                        .clicked()
                    {
                        self.erreur = systeme::montrer_le_fichier(&self.chemin).err();
                    }
                    if ui.button("📋 Copier le chemin").clicked() {
                        ui.ctx().copy_text(self.chemin.display().to_string());
                        self.copie = true;
                    }
                    if self.copie {
                        ui.weak("Copié !");
                    }
                });
                if let Some(erreur) = &self.erreur {
                    ui.colored_label(ui.visuals().error_fg_color, erreur);
                }
                ui.add_space(4.0);
                if ui.button("Fermer").clicked() {
                    fermer = true;
                }
            });
        ouverte && !fermer
    }

    fn grille(&self, ui: &mut egui::Ui) {
        let ligne = |ui: &mut egui::Ui, nom: &str, valeur: &str| {
            ui.weak(nom);
            ui.add(egui::Label::new(valeur).wrap());
            ui.end_row();
        };
        match &self.infos {
            None => {
                ui.spinner();
                ui.label("Lecture du fichier…");
                ui.end_row();
            }
            Some(Err(erreur)) => {
                ui.weak("Erreur");
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Fichier illisible : {erreur}"),
                );
                ui.end_row();
            }
            Some(Ok(infos)) => {
                match &infos.qualite {
                    Some(q) => {
                        ligne(ui, "Débit", &q.debit.texte_complet());
                        ligne(ui, "Fréquence", &texte_frequence(q.frequence));
                        ligne(ui, "Canaux", q.canaux.texte());
                        ligne(ui, "Durée", &crate::format_temps(q.duree));
                        ligne(ui, "Format", &format!("MP3 ({} Layer III)", q.version));
                    }
                    None => ligne(ui, "Format", "Pas de son MP3 reconnu dans ce fichier"),
                }
                let taille = format!(
                    "{} ({} octets)",
                    texte_taille(infos.taille),
                    avec_espaces(infos.taille)
                );
                ligne(ui, "Taille", &taille);
            }
        }
        ligne(ui, "Fichier", &nom_du_fichier(&self.chemin));
        let dossier = self
            .chemin
            .parent()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        ligne(ui, "Dossier", &dossier);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un en-tête MPEG-1 Layer III, 44,1 kHz, stéréo jointe, au numéro de
    /// débit donné (9 = 128 kbit/s, 14 = 320 kbit/s), sans remplissage.
    fn entete(numero_debit: u8, mode: u8) -> [u8; 4] {
        [0xFF, 0xFB, numero_debit << 4, mode << 6]
    }

    /// Une trame entière : son en-tête suivi de zéros.
    fn trame(numero_debit: u8, mode: u8) -> Vec<u8> {
        let e = entete(numero_debit, mode);
        let longueur = Entete::lire(&e).unwrap().longueur;
        let mut t = e.to_vec();
        t.resize(longueur, 0);
        t
    }

    #[test]
    fn lit_un_entete() {
        let e = Entete::lire(&entete(9, 1)).unwrap();
        assert_eq!(e.debit, 128);
        assert_eq!(e.frequence, 44_100);
        assert_eq!(e.canaux, Canaux::StereoJointe);
        // 144 × 128 000 / 44 100 = 417 octets.
        assert_eq!(e.longueur, 417);
        assert!(Entete::lire(&[0xFF, 0xFB, 0xF0, 0]).is_none());
        assert!(Entete::lire(b"ID3\x04").is_none());
    }

    #[test]
    fn debit_constant() {
        let mut octets = Vec::new();
        for _ in 0..100 {
            octets.extend(trame(14, 1));
        }
        let q = analyser(&octets).unwrap();
        assert_eq!(q.debit, Debit::Constant(320));
        assert_eq!(q.frequence, 44_100);
        // 100 trames de 1152 échantillons à 44 100 Hz ≈ 2,6 s.
        assert_eq!(q.duree.as_millis(), 2612);
    }

    #[test]
    fn debit_variable_donne_la_moyenne() {
        let mut octets = Vec::new();
        for _ in 0..50 {
            octets.extend(trame(9, 1)); // 128 kbit/s
            octets.extend(trame(14, 1)); // 320 kbit/s
        }
        let Debit::Variable(moyen) = analyser(&octets).unwrap().debit else {
            panic!("le débit devrait être variable");
        };
        assert!((223..=225).contains(&moyen), "{moyen}");
    }

    #[test]
    fn saute_les_tags_id3_et_l_en_tete_xing() {
        // Un tag ID3v2 de 20 octets (dont des 0xFF qui ne sont pas une trame).
        let mut octets = b"ID3\x03\x00\x00\x00\x00\x00\x14".to_vec();
        octets.extend([0xFF; 20]);
        // Une trame « Info » à 128 kbit/s (sans son), puis du 320 constant.
        let mut info = trame(9, 1);
        info[36..40].copy_from_slice(b"Info");
        octets.extend(info);
        for _ in 0..10 {
            octets.extend(trame(14, 1));
        }
        // Un tag ID3v1 à la fin.
        let mut v1 = b"TAG".to_vec();
        v1.resize(128, b' ');
        octets.extend(v1);
        let q = analyser(&octets).unwrap();
        assert_eq!(q.debit, Debit::Constant(320));
        assert_eq!(q.duree.as_millis(), 261);
    }

    #[test]
    fn mono_et_mpeg2() {
        // MPEG-2 (version 10), 22 050 Hz, 64 kbit/s (numéro 8), mono.
        let e = [0xFF, 0xF3, 8 << 4, 3 << 6];
        let longueur = Entete::lire(&e).unwrap().longueur;
        let mut octets = Vec::new();
        for _ in 0..5 {
            octets.extend(e);
            octets.resize(octets.len() + longueur - 4, 0);
        }
        let q = analyser(&octets).unwrap();
        assert_eq!(q.debit, Debit::Constant(64));
        assert_eq!(q.frequence, 22_050);
        assert_eq!(q.canaux, Canaux::Mono);
        assert_eq!(q.version, "MPEG-2");
    }

    #[test]
    fn pas_de_mp3() {
        assert_eq!(analyser(b"bonjour, je ne suis pas un MP3"), None);
        assert_eq!(analyser(&[]), None);
    }

    #[test]
    fn textes() {
        assert_eq!(texte_frequence(44_100), "44,1 kHz");
        assert_eq!(texte_frequence(48_000), "48 kHz");
        assert_eq!(texte_taille(8_812_345), "8,4 Mo");
        assert_eq!(texte_taille(524_288), "512 Ko");
        assert_eq!(texte_taille(12), "12 octets");
        assert_eq!(avec_espaces(8_812_345), "8\u{202F}812\u{202F}345");
        let infos = Infos {
            taille: 8_812_345,
            qualite: Some(Qualite {
                debit: Debit::Constant(320),
                frequence: 44_100,
                canaux: Canaux::StereoJointe,
                version: "MPEG-1",
                duree: Duration::from_secs(200),
            }),
        };
        assert_eq!(
            resume(&infos),
            "MP3 · 320 kbit/s · 44,1 kHz · stéréo · 8,4 Mo"
        );
    }
}
