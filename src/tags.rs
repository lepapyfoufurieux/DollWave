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
        let mut tags = Tags {
            titre: texte(StandardTagKey::TrackTitle),
            artiste: texte(StandardTagKey::Artist),
            album: texte(StandardTagKey::Album),
            duree,
            recherche: String::new(),
        };
        tags.preparer_la_recherche();
        tags
    })
    .unwrap_or_default()
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

    pub fn album(&self, chemin: &Path) -> Option<String> {
        self.avec(chemin, |tags| tags.and_then(|t| t.album.clone()))
    }

    pub fn duree(&self, chemin: &Path) -> Option<Duration> {
        self.avec(chemin, |tags| tags.and_then(|t| t.duree))
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
        let mut cadres = Vec::new();
        for (cle, valeur) in [("TIT2", titre), ("TPE1", artiste), ("TALB", album)] {
            if let Some(valeur) = valeur {
                cadres.extend(cadre_texte(cle, valeur));
            }
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
}
