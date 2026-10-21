//! Les tags ID3 : titre, artiste, album et pochette rangés dans le fichier
//! MP3, plus sa durée. On les lit avec symphonia, le décodeur que rodio
//! utilise déjà pour la musique.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, StandardVisualKey};
use symphonia::core::probe::Hint;

use crate::egalisation::Mesure;
use crate::liste::nom_du_fichier;

/// Côté le plus long de la pochette une fois réduite : assez pour qu'elle
/// reste nette à l'écran, sans garder une image de 3000 × 3000 en mémoire.
const TAILLE_POCHETTE: u32 = 640;

/// Pendant la lecture des tags en arrière-plan, on redessine la fenêtre
/// au plus toutes les 200 ms (et pas après chaque fichier).
const PAUSE_ENTRE_AFFICHAGES: Duration = Duration::from_millis(200);

/// Ce qu'on sait d'un morceau. Chaque information peut manquer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub titre: Option<String>,
    pub artiste: Option<String>,
    pub album: Option<String>,
    /// L'« artiste de l'album », s'il est indiqué : sur un album avec des
    /// invités (« feat. »), il reste le même pour tous les morceaux.
    pub artiste_album: Option<String>,
    /// La place du morceau sur l'album, et le numéro du disque.
    pub numero: Option<u32>,
    pub disque: Option<u32>,
    pub duree: Option<Duration>,
    /// Titre, artiste et album en minuscules, préparés une fois pour toutes
    /// pour la recherche (voir `texte_a_chercher`).
    recherche: String,
}

impl Tags {
    /// Le nom à afficher : « Artiste – Titre », le titre seul s'il n'y a pas
    /// d'artiste, et le nom du fichier s'il n'y a pas de titre.
    pub fn libelle(&self, chemin: &Path) -> String {
        match (&self.artiste, &self.titre) {
            (Some(artiste), Some(titre)) => format!("{artiste} – {titre}"),
            (None, Some(titre)) => titre.clone(),
            _ => nom_du_fichier(chemin),
        }
    }

    /// Le titre, l'artiste et l'album à la suite, en minuscules, pour la recherche.
    pub fn texte_a_chercher(&self) -> &str {
        &self.recherche
    }

    fn preparer_la_recherche(&mut self) {
        self.recherche = [&self.titre, &self.artiste, &self.album]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
}

/// Lit les tags d'un fichier. Un fichier illisible ou sans tags donne des
/// tags vides : on affichera alors son nom de fichier.
pub fn lire(chemin: &Path) -> Tags {
    ouvrir(chemin, |revision, duree| {
        let texte = |cle: StandardTagKey| -> Option<String> {
            let tag = revision?.tags().iter().find(|t| t.std_key == Some(cle))?;
            let valeur = tag.value.to_string();
            let valeur = valeur.trim();
            (!valeur.is_empty()).then(|| valeur.to_string())
        };
        let nombre = |cle: StandardTagKey| texte(cle).and_then(|t| premier_nombre(&t));
        let mut tags = Tags {
            titre: texte(StandardTagKey::TrackTitle),
            artiste: texte(StandardTagKey::Artist),
            album: texte(StandardTagKey::Album),
            artiste_album: texte(StandardTagKey::AlbumArtist),
            numero: nombre(StandardTagKey::TrackNumber),
            disque: nombre(StandardTagKey::DiscNumber),
            duree,
            recherche: String::new(),
        };
        tags.preparer_la_recherche();
        tags
    })
    .unwrap_or_default()
}

/// Le volume du morceau d'après ses tags ReplayGain « par morceau », s'il
/// en a (« -6.54 dB » pour le gain, « 0.98 » pour la crête). Sans crête
/// indiquée, on la suppose au maximum : le morceau ne sera jamais monté.
pub fn lire_replaygain(chemin: &Path) -> Option<Mesure> {
    ouvrir(chemin, |revision, _| {
        let tags = revision?.tags();
        // Selon le logiciel qui les a écrits, ces tags sont en majuscules
        // ou en minuscules : on les cherche par leur nom, sans distinction.
        let valeur = |cle: StandardTagKey, nom: &str| -> Option<f32> {
            let tag = tags
                .iter()
                .find(|t| t.std_key == Some(cle) || t.key.to_ascii_lowercase().ends_with(nom))?;
            premier_decimal(&tag.value.to_string())
        };
        let gain_db = valeur(StandardTagKey::ReplayGainTrackGain, "replaygain_track_gain")?;
        let crete = valeur(StandardTagKey::ReplayGainTrackPeak, "replaygain_track_peak")
            .filter(|c| *c > 0.0)
            .unwrap_or(1.0);
        Some(Mesure { gain_db, crete })
    })?
}

/// Le nombre décimal au début d'un tag : « -6.54 dB » donne -6.54.
fn premier_decimal(texte: &str) -> Option<f32> {
    let texte = texte.trim().replace(',', ".");
    let fin = texte
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && (c == '-' || c == '+'))))
        .map_or(texte.len(), |(i, _)| i);
    texte[..fin].parse().ok().filter(|v: &f32| v.is_finite())
}

/// Le nombre au début d'un tag : « 3/12 » (piste 3 sur 12) donne 3.
fn premier_nombre(texte: &str) -> Option<u32> {
    let chiffres: String = texte
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    chiffres.parse().ok()
}

/// La pochette rangée dans le fichier (de préférence celle marquée « devant »),
/// réduite pour l'affichage. Rien si le fichier n'en a pas.
pub fn lire_pochette(chemin: &Path) -> Option<egui::ColorImage> {
    let donnees = ouvrir(chemin, |revision, _| {
        let visuels = revision?.visuals();
        let visuel = visuels
            .iter()
            .find(|v| v.usage == Some(StandardVisualKey::FrontCover))
            .or_else(|| visuels.first())?;
        Some(visuel.data.to_vec())
    })??;
    let mut image = image::load_from_memory(&donnees).ok()?;
    if image.width().max(image.height()) > TAILLE_POCHETTE {
        image = image.thumbnail(TAILLE_POCHETTE, TAILLE_POCHETTE);
    }
    let image = image.to_rgba8();
    let taille = [image.width() as usize, image.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        taille,
        image.as_raw(),
    ))
}

/// Ouvre le fichier avec symphonia, sans décoder la musique, et passe ses
/// tags et sa durée à `lire`. Rien si ce n'est pas un MP3 lisible.
fn ouvrir<T>(
    chemin: &Path,
    lire: impl FnOnce(Option<&MetadataRevision>, Option<Duration>) -> T,
) -> Option<T> {
    let fichier = File::open(chemin).ok()?;
    let source = MediaSourceStream::new(Box::new(fichier), Default::default());
    let mut indice = Hint::new();
    indice.with_extension("mp3");
    let mut sonde = symphonia::default::get_probe()
        .format(
            &indice,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    // La durée : le nombre d'échantillons divisé par leur nombre par seconde.
    // Pour les MP3 qui ne l'indiquent pas, symphonia l'estime d'après la
    // taille du fichier.
    let duree = sonde.format.default_track().and_then(|piste| {
        let parametres = &piste.codec_params;
        let echantillons = parametres.n_frames?;
        let par_seconde = parametres.sample_rate?;
        Some(Duration::from_secs_f64(
            echantillons as f64 / f64::from(par_seconde),
        ))
    });
    // Les tags ID3v2 sont au début du fichier : symphonia les lit en
    // reconnaissant le format. Sinon, on regarde ceux trouvés dans le MP3 lui-même.
    if let Some(debut) = sonde.metadata.get() {
        if let Some(revision) = debut.current() {
            return Some(lire(Some(revision), duree));
        }
    }
    Some(lire(sonde.format.metadata().current(), duree))
}

/// Les tags déjà lus, pour la bibliothèque et la liste de lecture. Les
/// fichiers sont lus en arrière-plan, un par un, pour que la fenêtre ne se
/// fige jamais ; en attendant, on affiche le nom du fichier.
pub struct TagsConnus {
    partage: Arc<(Mutex<Etat>, Condvar)>,
}

#[derive(Default)]
struct Etat {
    lus: HashMap<PathBuf, Tags>,
    /// Les fichiers demandés mais pas encore lus (en attente, ou en cours de
    /// lecture), pour ne pas les demander deux fois. Une fois lus, ils sont
    /// dans `lus` : on ne garde pas chaque chemin en double.
    en_attente: HashSet<PathBuf>,
    /// Les fichiers qui attendent d'être lus, dans l'ordre.
    a_lire: VecDeque<PathBuf>,
    /// Augmente chaque fois que de nouveaux tags sont affichés : la
    /// recherche sait ainsi qu'elle doit se refaire.
    version: u64,
}

impl TagsConnus {
    /// Lance le lecteur de tags en arrière-plan.
    pub fn new(ctx: &egui::Context) -> Self {
        let partage = Arc::new((Mutex::new(Etat::default()), Condvar::new()));
        let pour_le_lecteur = Arc::clone(&partage);
        let ctx = ctx.clone();
        std::thread::spawn(move || lire_en_arriere_plan(&pour_le_lecteur, &ctx));
        Self { partage }
    }

    fn etat(&self) -> std::sync::MutexGuard<'_, Etat> {
        // Si le lecteur avait planté en tenant le verrou, on continue quand même.
        self.partage.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Demande la lecture de ces fichiers, après ceux déjà demandés (pour
    /// toute la bibliothèque, afin que la recherche trouve les tags).
    pub fn demander<'a>(&self, chemins: impl IntoIterator<Item = &'a Path>) {
        let mut etat = self.etat();
        for chemin in chemins {
            if !etat.lus.contains_key(chemin) && etat.en_attente.insert(chemin.to_path_buf()) {
                etat.a_lire.push_back(chemin.to_path_buf());
            }
        }
        self.partage.1.notify_one();
    }

    /// Passe à `f` les tags du fichier s'ils sont déjà lus. Sinon, le
    /// fichier passe en tête de la file : il est à l'écran, on le veut vite.
    fn avec<T>(&self, chemin: &Path, f: impl FnOnce(Option<&Tags>) -> T) -> T {
        let mut etat = self.etat();
        // On vérifie d'abord sans rien copier : c'est le cas courant, à
        // chaque image pour chaque ligne affichée.
        if !etat.lus.contains_key(chemin) && !etat.en_attente.contains(chemin) {
            etat.en_attente.insert(chemin.to_path_buf());
            etat.a_lire.push_front(chemin.to_path_buf());
            self.partage.1.notify_one();
        }
        f(etat.lus.get(chemin))
    }

    /// Relit ce fichier dès que possible (ses tags viennent d'être modifiés).
    /// En attendant, on garde les anciens tags à l'écran.
    pub fn relire(&self, chemin: &Path) {
        let mut etat = self.etat();
        if !etat.en_attente.insert(chemin.to_path_buf()) {
            // Déjà dans la file : on le remet en tête.
            etat.a_lire.retain(|c| c != chemin);
        }
        etat.a_lire.push_front(chemin.to_path_buf());
        self.partage.1.notify_one();
    }

    /// Le nom à afficher pour ce morceau (voir `Tags::libelle`).
    pub fn libelle(&self, chemin: &Path) -> String {
        self.avec(chemin, |tags| match tags {
            Some(tags) => tags.libelle(chemin),
            None => nom_du_fichier(chemin),
        })
    }

    /// Le texte de l'infobulle : l'album (s'il est connu) et le nom du fichier.
    pub fn infobulle(&self, chemin: &Path) -> String {
        let album = self.avec(chemin, |tags| tags.and_then(|t| t.album.clone()));
        match album {
            Some(album) => format!("Album : {album}\n{}", nom_du_fichier(chemin)),
            None => nom_du_fichier(chemin),
        }
    }

    /// Le titre (le nom du fichier sans « .mp3 » s'il n'en a pas) et l'artiste,
    /// séparés, pour le bloc « En lecture ».
    pub fn titre_et_artiste(&self, chemin: &Path) -> (String, Option<String>) {
        self.avec(chemin, |tags| {
            let titre = tags.and_then(|t| t.titre.clone()).unwrap_or_else(|| {
                chemin
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            (titre, tags.and_then(|t| t.artiste.clone()))
        })
    }

    pub fn album(&self, chemin: &Path) -> Option<String> {
        self.avec(chemin, |tags| tags.and_then(|t| t.album.clone()))
    }

    pub fn duree(&self, chemin: &Path) -> Option<Duration> {
        self.avec(chemin, |tags| tags.and_then(|t| t.duree))
    }

    /// La durée totale de ces morceaux, une fois qu'ils sont tous lus (rien
    /// avant, pour ne pas afficher un total faux qui grandit). Un morceau
    /// dont on n'a pas pu lire la durée compte pour zéro.
    pub fn duree_totale(&self, chemins: &[PathBuf]) -> Option<Duration> {
        let mut etat = self.etat();
        let mut total = Duration::ZERO;
        let mut tout_lu = true;
        for chemin in chemins {
            match etat.lus.get(chemin) {
                Some(tags) => total += tags.duree.unwrap_or_default(),
                None => {
                    tout_lu = false;
                    // Hors de l'écran, il n'a peut-être pas encore été demandé.
                    if etat.en_attente.insert(chemin.clone()) {
                        etat.a_lire.push_back(chemin.clone());
                        self.partage.1.notify_one();
                    }
                }
            }
        }
        tout_lu.then_some(total)
    }

    /// De quoi chercher dans les tags de toute la bibliothèque, en ne prenant
    /// le verrou qu'une fois pour toute la recherche (et pas une fois par
    /// morceau). Pas de `avec` ici : la bibliothèque est déjà demandée en entier.
    pub fn pour_chercher(&self) -> TextesAChercher<'_> {
        TextesAChercher(self.etat())
    }

    pub fn version(&self) -> u64 {
        self.etat().version
    }
}

/// Les tags connus, le temps d'une recherche (voir `TagsConnus::pour_chercher`).
pub struct TextesAChercher<'a>(std::sync::MutexGuard<'a, Etat>);

impl TextesAChercher<'_> {
    /// Les tags du morceau, s'ils sont déjà lus.
    pub fn tags(&self, chemin: &Path) -> Option<&Tags> {
        self.0.lus.get(chemin)
    }

    /// Titre, artiste et album en minuscules (vide tant que ce n'est pas lu).
    pub fn de(&self, chemin: &Path) -> &str {
        self.0.lus.get(chemin).map_or("", Tags::texte_a_chercher)
    }
}

/// La boucle du lecteur de tags : prend le prochain fichier à lire, le lit
/// (sans tenir le verrou, pour ne pas bloquer l'affichage) et range le résultat.
fn lire_en_arriere_plan(partage: &(Mutex<Etat>, Condvar), ctx: &egui::Context) {
    let (verrou, signal) = partage;
    let mut dernier_affichage = Instant::now();
    let mut a_montrer = false;
    loop {
        let suivant = {
            let mut etat = verrou.lock().unwrap_or_else(|e| e.into_inner());
            // Plus rien à lire : on montre ce qui reste, puis on attend.
            if etat.a_lire.is_empty() && a_montrer {
                etat.version += 1;
                a_montrer = false;
                ctx.request_repaint();
            }
            while etat.a_lire.is_empty() {
                etat = signal.wait(etat).unwrap_or_else(|e| e.into_inner());
            }
            etat.a_lire.pop_front()
        };
        let Some(chemin) = suivant else { continue };
        let tags = lire(&chemin);
        let mut etat = verrou.lock().unwrap_or_else(|e| e.into_inner());
        etat.en_attente.remove(&chemin);
        etat.lus.insert(chemin, tags);
        a_montrer = true;
        if dernier_affichage.elapsed() >= PAUSE_ENTRE_AFFICHAGES {
            etat.version += 1;
            a_montrer = false;
            dernier_affichage = Instant::now();
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Un nombre écrit sur 4 octets de 7 bits, comme dans les tags ID3v2.4.
    fn taille_id3(n: usize) -> [u8; 4] {
        [
            (n >> 21) as u8 & 0x7f,
            (n >> 14) as u8 & 0x7f,
            (n >> 7) as u8 & 0x7f,
            n as u8 & 0x7f,
        ]
    }

    /// Un cadre de tag ID3v2.4 : son nom (« TIT2 »…) puis son contenu.
    fn cadre(nom: &str, contenu: &[u8]) -> Vec<u8> {
        let mut cadre = nom.as_bytes().to_vec();
        cadre.extend(taille_id3(contenu.len()));
        cadre.extend([0, 0]);
        cadre.extend(contenu);
        cadre
    }

    /// Un cadre de texte, en UTF-8.
    fn cadre_texte(nom: &str, texte: &str) -> Vec<u8> {
        let mut contenu = vec![3];
        contenu.extend(texte.as_bytes());
        cadre(nom, &contenu)
    }

    /// Une petite image PNG de 4 × 2 pixels rouges, comme pochette.
    fn png_rouge() -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        png.into_inner()
    }

    /// Écrit un faux MP3 : ces tags, puis `trames` trames MP3 silencieuses
    /// (128 kbit/s, 44,1 kHz, 1152 échantillons chacune).
    pub fn ecrire_mp3(
        nom: &str,
        titre: Option<&str>,
        artiste: Option<&str>,
        album: Option<&str>,
        pochette: bool,
        trames: usize,
    ) -> PathBuf {
        let textes: Vec<(&str, &str)> = [("TIT2", titre), ("TPE1", artiste), ("TALB", album)]
            .into_iter()
            .filter_map(|(cle, valeur)| Some((cle, valeur?)))
            .collect();
        ecrire_mp3_avec(nom, &textes, pochette, trames)
    }

    /// Comme `ecrire_mp3`, avec n'importe quels cadres de texte
    /// (« TPE2 » pour l'artiste de l'album, « TRCK » pour la piste…).
    pub fn ecrire_mp3_avec(
        nom: &str,
        textes: &[(&str, &str)],
        pochette: bool,
        trames: usize,
    ) -> PathBuf {
        let mut cadres = Vec::new();
        for (cle, valeur) in textes {
            cadres.extend(cadre_texte(cle, valeur));
        }
        if pochette {
            // Encodage, type d'image, « 3 » = pochette de devant, description vide.
            let mut contenu = vec![0];
            contenu.extend(b"image/png\0");
            contenu.extend([3, 0]);
            contenu.extend(png_rouge());
            cadres.extend(cadre("APIC", &contenu));
        }
        let mut fichier = Vec::new();
        if !cadres.is_empty() {
            fichier.extend(b"ID3\x04\x00\x00");
            fichier.extend(taille_id3(cadres.len()));
            fichier.extend(cadres);
        }
        for _ in 0..trames {
            let mut trame = vec![0u8; 417];
            trame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
            fichier.extend(trame);
        }
        let dossier = std::env::temp_dir().join("lecteur_mp3_tests_tags");
        std::fs::create_dir_all(&dossier).unwrap();
        let chemin = dossier.join(nom);
        std::fs::write(&chemin, fichier).unwrap();
        chemin
    }

    #[test]
    fn lit_titre_artiste_album_et_duree() {
        let chemin = ecrire_mp3(
            "complet.mp3",
            Some("One More Time"),
            Some("Daft Punk"),
            Some("Discovery"),
            false,
            100,
        );
        let tags = lire(&chemin);
        assert_eq!(tags.titre.as_deref(), Some("One More Time"));
        assert_eq!(tags.artiste.as_deref(), Some("Daft Punk"));
        assert_eq!(tags.album.as_deref(), Some("Discovery"));
        // 100 trames de 1152 échantillons à 44 100 par seconde : 2,6 s.
        let duree = tags.duree.unwrap().as_secs_f32();
        assert!((duree - 2.61).abs() < 0.1, "durée lue : {duree}");
        assert_eq!(tags.libelle(&chemin), "Daft Punk – One More Time");
        assert_eq!(tags.texte_a_chercher(), "one more time daft punk discovery");
    }

    #[test]
    fn lit_l_artiste_de_l_album_la_piste_et_le_disque() {
        let chemin = ecrire_mp3_avec(
            "piste.mp3",
            &[
                ("TIT2", "Harder"),
                ("TPE1", "Daft Punk feat. Moi"),
                ("TPE2", "Daft Punk"),
                ("TRCK", "4/14"),
                ("TPOS", "2"),
            ],
            false,
            50,
        );
        let tags = lire(&chemin);
        assert_eq!(tags.artiste_album.as_deref(), Some("Daft Punk"));
        assert_eq!(tags.numero, Some(4));
        assert_eq!(tags.disque, Some(2));
        // Sans ces tags : rien.
        let chemin = ecrire_mp3("sans piste.mp3", Some("Intro"), None, None, false, 50);
        let tags = lire(&chemin);
        assert_eq!(
            (tags.artiste_album, tags.numero, tags.disque),
            (None, None, None)
        );
    }

    /// Un faux MP3 avec un titre et ces cadres TXXX (description, valeur).
    fn ecrire_mp3_txxx(nom: &str, txxx: &[(&str, &str)]) -> PathBuf {
        let textes: Vec<String> = txxx.iter().map(|(d, v)| format!("{d}\0{v}")).collect();
        let mut cadres = vec![("TIT2", "Titre")];
        cadres.extend(textes.iter().map(|t| ("TXXX", t.as_str())));
        ecrire_mp3_avec(nom, &cadres, false, 100)
    }

    #[test]
    fn lit_les_tags_replaygain() {
        let chemin = ecrire_mp3_txxx(
            "replaygain.mp3",
            &[
                ("REPLAYGAIN_TRACK_GAIN", "-6.54 dB"),
                ("REPLAYGAIN_TRACK_PEAK", "0.987654"),
                ("REPLAYGAIN_ALBUM_GAIN", "-8.00 dB"),
            ],
        );
        let mesure = lire_replaygain(&chemin).expect("tags ReplayGain");
        assert_eq!(mesure.gain_db, -6.54);
        assert_eq!(mesure.crete, 0.987654);
        // Les autres tags sont toujours lus.
        assert_eq!(lire(&chemin).titre.as_deref(), Some("Titre"));
    }

    /// En minuscules et sans crête (certains logiciels) : on suppose la
    /// crête au maximum.
    #[test]
    fn replaygain_en_minuscules_sans_crete() {
        let chemin = ecrire_mp3_txxx(
            "replaygain_min.mp3",
            &[("replaygain_track_gain", "+3.2 dB")],
        );
        let mesure = lire_replaygain(&chemin).expect("tags ReplayGain");
        assert_eq!(mesure.gain_db, 3.2);
        assert_eq!(mesure.crete, 1.0);
    }

    #[test]
    fn sans_tags_replaygain() {
        let chemin = ecrire_mp3("sans_replaygain.mp3", Some("T"), None, None, false, 100);
        assert_eq!(lire_replaygain(&chemin), None);
        let album_seul = ecrire_mp3_txxx("album_seul.mp3", &[("REPLAYGAIN_ALBUM_GAIN", "-8 dB")]);
        assert_eq!(lire_replaygain(&album_seul), None);
    }

    #[test]
    fn premier_decimal_d_un_tag() {
        assert_eq!(premier_decimal("-6.54 dB"), Some(-6.54));
        assert_eq!(premier_decimal(" +3,5 dB"), Some(3.5));
        assert_eq!(premier_decimal("0.98"), Some(0.98));
        assert_eq!(premier_decimal("dB"), None);
        assert_eq!(premier_decimal(""), None);
    }

    #[test]
    fn premier_nombre_d_un_tag() {
        assert_eq!(premier_nombre("3/12"), Some(3));
        assert_eq!(premier_nombre(" 07 "), Some(7));
        assert_eq!(premier_nombre("A1"), None);
        assert_eq!(premier_nombre(""), None);
    }

    #[test]
    fn sans_tags_on_garde_le_nom_du_fichier() {
        let chemin = ecrire_mp3("sans tags.mp3", None, None, None, false, 50);
        let tags = lire(&chemin);
        assert_eq!(
            (&tags.titre, &tags.artiste, &tags.album),
            (&None, &None, &None)
        );
        assert!(tags.duree.is_some());
        assert_eq!(tags.libelle(&chemin), "sans tags.mp3");
        assert!(lire_pochette(&chemin).is_none());
    }

    #[test]
    fn titre_seul_sans_artiste() {
        let chemin = ecrire_mp3("titre seul.mp3", Some("Intro"), None, None, false, 50);
        assert_eq!(lire(&chemin).libelle(&chemin), "Intro");
    }

    #[test]
    fn fichier_illisible() {
        let dossier = std::env::temp_dir().join("lecteur_mp3_tests_tags");
        std::fs::create_dir_all(&dossier).unwrap();
        let chemin = dossier.join("pas un mp3.mp3");
        std::fs::write(&chemin, b"bonjour").unwrap();
        assert_eq!(lire(&chemin), Tags::default());
        assert_eq!(lire(&dossier.join("introuvable.mp3")), Tags::default());
    }

    #[test]
    fn lit_la_pochette() {
        let chemin = ecrire_mp3("pochette.mp3", Some("Avec image"), None, None, true, 50);
        let pochette = lire_pochette(&chemin).expect("pochette non trouvée");
        assert_eq!(pochette.size, [4, 2]);
        assert_eq!(pochette.pixels[0], egui::Color32::RED);
        // La pochette ne gêne pas la lecture des autres tags.
        assert_eq!(lire(&chemin).titre.as_deref(), Some("Avec image"));
    }

    #[test]
    fn lecture_en_arriere_plan() {
        let chemin = ecrire_mp3(
            "arriere plan.mp3",
            Some("Fond"),
            Some("Moi"),
            None,
            false,
            50,
        );
        let connus = TagsConnus::new(&egui::Context::default());
        // Pas encore lu : le nom du fichier, et la lecture est lancée.
        assert_eq!(connus.libelle(&chemin), "arriere plan.mp3");
        let debut = Instant::now();
        while connus.libelle(&chemin) != "Moi – Fond" {
            assert!(debut.elapsed() < Duration::from_secs(5), "tags jamais lus");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(connus.pour_chercher().de(&chemin), "fond moi");
        // Une fois lu, le chemin n'est plus gardé en double, et le
        // redemander ne relance pas sa lecture.
        assert!(connus.etat().en_attente.is_empty());
        connus.demander([chemin.as_path()]);
        assert!(connus.etat().a_lire.is_empty());
        assert!(connus.etat().en_attente.is_empty());
    }

    #[test]
    fn duree_totale_une_fois_tout_lu() {
        // 100 trames ≈ 2,6 s et 50 trames ≈ 1,3 s ; plus un fichier
        // illisible, qui compte pour zéro sans bloquer le total.
        let a = ecrire_mp3("total a.mp3", None, None, None, false, 100);
        let b = ecrire_mp3("total b.mp3", None, None, None, false, 50);
        let dossier = std::env::temp_dir().join("lecteur_mp3_tests_tags");
        let illisible = dossier.join("total illisible.mp3");
        std::fs::write(&illisible, b"bonjour").unwrap();
        let chemins = vec![a, b, illisible];
        let connus = TagsConnus::new(&egui::Context::default());
        assert_eq!(connus.duree_totale(&[]), Some(Duration::ZERO));
        let debut = Instant::now();
        let total = loop {
            if let Some(total) = connus.duree_totale(&chemins) {
                break total.as_secs_f32();
            }
            assert!(
                debut.elapsed() < Duration::from_secs(5),
                "durées jamais lues"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!((total - 3.92).abs() < 0.1, "total : {total}");
    }
}
