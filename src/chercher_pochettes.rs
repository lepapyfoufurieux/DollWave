//! Le bouton « 🖼 » de la bibliothèque : pour chaque album dont des MP3
//! n'ont pas de pochette, on en cherche une et on l'écrit dans leurs tags.
//!
//! Album par album :
//! 1. si un de ses morceaux a déjà une pochette, on la recopie dans les
//!    autres (sans Internet) ;
//! 2. sinon on cherche l'album sur MusicBrainz, et sa pochette sur le
//!    Cover Art Archive (gratuits, sans compte) ;
//! 3. si rien n'est trouvé, on essaie Deezer puis iTunes (gratuits, sans
//!    compte).
//!
//! Un morceau sans album est cherché tout seul, par son artiste et son
//! titre : on prend la pochette de l'album ou du single où il est paru.
//!
//! L'image est rangée dans le MP3, comme le fait « Modifier les tags » (voir
//! `ecriture_tags` : copie vérifiée, puis remplacement). Une pochette déjà
//! présente n'est jamais remplacée, et rien n'est ajouté dans le dossier.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use eframe::egui;

use crate::ecriture_tags::{self, Modifications, Pochette};
use crate::generer_paroles::{self, normaliser, ressemblance};
use crate::introuvables::{self, Introuvables};
use crate::reseau::{self, Echec};
use crate::tags::{self, Tags};

const MUSICBRAINZ: &str = "https://musicbrainz.org/ws/2/release-group/";
const ITUNES: &str = "https://itunes.apple.com/search";
const DEEZER_ALBUMS: &str = "https://api.deezer.com/search/album";
const DEEZER_MORCEAUX: &str = "https://api.deezer.com/search/track";
/// MusicBrainz accepte une recherche par seconde au plus.
const PAUSE_ENTRE_RECHERCHES: Duration = Duration::from_millis(1100);
/// Au-delà de ces ressemblances, la réponse est bien le même album.
const TITRE_MIN: f64 = 0.85;
const ARTISTE_MIN: f64 = 0.6;
/// On essaie au plus autant d'images par source (une réponse sans pochette
/// est fréquente sur le Cover Art Archive).
const IMAGES_ESSAYEES: usize = 3;
/// Après autant d'albums de suite sans réussir à joindre Internet, on
/// s'arrête : la connexion est sans doute coupée.
const ECHECS_DE_SUITE_MAX: usize = 3;

// ---------------------------------------------------------------- les albums

/// Un album dont au moins un MP3 n'a pas de pochette (ou un morceau sans
/// album, voir `morceau`).
#[derive(Debug, PartialEq)]
pub struct AlbumACompleter {
    pub album: String,
    /// Pour un morceau sans album : son titre. On cherche alors le morceau,
    /// et pas l'album.
    pub morceau: Option<String>,
    /// L'artiste de l'album, s'il est connu (rien pour une compilation).
    pub artiste: Option<String>,
    /// Les MP3 sans pochette.
    pub sans_pochette: Vec<PathBuf>,
    /// Un MP3 de l'album qui a déjà une pochette, s'il y en a un.
    pub avec_pochette: Option<PathBuf>,
}

impl AlbumACompleter {
    /// « Album — Artiste », pour la progression et le résumé.
    pub fn libelle(&self) -> String {
        let nom = self.morceau.as_ref().unwrap_or(&self.album);
        match &self.artiste {
            Some(artiste) => format!("{nom} — {artiste}"),
            None => nom.clone(),
        }
    }
}

/// Ce qu'on sait d'un morceau pour le ranger dans son album.
pub struct MorceauLu {
    pub chemin: PathBuf,
    pub tags: Tags,
    pub a_une_pochette: bool,
}

/// Les albums à compléter, et le nombre d'albums déjà complets. Comme dans
/// la vue « Albums » : même nom d'album et même artiste de l'album (à
/// défaut, même dossier, pour une compilation). Les morceaux sans album et
/// sans pochette sont cherchés un par un, après les albums.
pub fn albums_a_completer(morceaux: Vec<MorceauLu>) -> (Vec<AlbumACompleter>, usize) {
    // Clé → (album, artistes vus, sans pochette, avec pochette).
    type Groupe = (String, Vec<String>, Vec<PathBuf>, Option<PathBuf>);
    let mut groupes: HashMap<(String, String), Groupe> = HashMap::new();
    let mut ordre = Vec::new();
    let mut seuls = Vec::new();
    for morceau in morceaux {
        let Some(album) = morceau.tags.album.clone() else {
            if !morceau.a_une_pochette {
                seuls.extend(morceau_seul(morceau));
            }
            continue;
        };
        let second = match &morceau.tags.artiste_album {
            Some(artiste) => normaliser(artiste),
            None => format!(
                "dossier:{}",
                morceau.chemin.parent().unwrap_or(Path::new("")).display()
            ),
        };
        let cle = (normaliser(&album), second);
        let groupe = groupes.entry(cle.clone()).or_insert_with(|| {
            ordre.push(cle);
            (album, Vec::new(), Vec::new(), None)
        });
        if let Some(artiste) = morceau
            .tags
            .artiste_album
            .as_ref()
            .or(morceau.tags.artiste.as_ref())
        {
            if !groupe
                .1
                .iter()
                .any(|a| normaliser(a) == normaliser(artiste))
            {
                groupe.1.push(artiste.clone());
            }
        }
        if morceau.a_une_pochette {
            groupe.3.get_or_insert(morceau.chemin);
        } else {
            groupe.2.push(morceau.chemin);
        }
    }
    let mut a_completer = Vec::new();
    let mut complets = 0;
    for cle in ordre {
        let (album, artistes, sans_pochette, avec_pochette) = groupes.remove(&cle).unwrap();
        if sans_pochette.is_empty() {
            complets += 1;
            continue;
        }
        a_completer.push(AlbumACompleter {
            album,
            morceau: None,
            // Plusieurs artistes sans « artiste de l'album » : une compilation.
            artiste: (artistes.len() == 1).then(|| artistes[0].clone()),
            sans_pochette,
            avec_pochette,
        });
    }
    a_completer.extend(seuls);
    (a_completer, complets)
}

/// Un morceau sans album, à chercher par son titre et son artiste (devinés
/// comme pour les paroles : tags, nom du fichier, dossier).
fn morceau_seul(morceau: MorceauLu) -> Option<AlbumACompleter> {
    let recherche = generer_paroles::preparer(&morceau.chemin, &morceau.tags);
    Some(AlbumACompleter {
        album: String::new(),
        morceau: Some(recherche.titres.first()?.clone()),
        artiste: recherche.artistes.first().cloned(),
        sans_pochette: vec![morceau.chemin],
        avec_pochette: None,
    })
}

// ---------------------------------------------------------------- chercher

/// Une pochette possible, trouvée en ligne.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidat {
    pub album: String,
    /// Le titre du morceau, quand on a cherché un morceau.
    pub titre: Option<String>,
    pub artiste: String,
    /// L'adresse de l'image.
    pub image: String,
}

/// Lit la réponse JSON d'une recherche MusicBrainz (groupes de parutions :
/// un album, toutes éditions confondues). L'image est celle du Cover Art
/// Archive, en 500 pixels.
pub fn lire_musicbrainz(json: &str) -> Vec<Candidat> {
    let Ok(reponse) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(groupes) = reponse.get("release-groups").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    groupes
        .iter()
        .filter_map(|g| {
            let id = g.get("id")?.as_str()?;
            let artiste = g
                .get("artist-credit")
                .and_then(|v| v.as_array())
                .map(|credits| {
                    credits
                        .iter()
                        .map(|c| {
                            let nom = c.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                            let liaison = c
                                .get("joinphrase")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            format!("{nom}{liaison}")
                        })
                        .collect::<String>()
                })
                .unwrap_or_default();
            Some(Candidat {
                album: g.get("title")?.as_str()?.to_string(),
                titre: None,
                artiste,
                image: format!("https://coverartarchive.org/release-group/{id}/front-500"),
            })
        })
        .collect()
}

/// Lit la réponse JSON d'une recherche iTunes (d'albums ou de morceaux).
/// L'image est demandée en 600 pixels (iTunes donne l'adresse de la
/// version 100 px).
pub fn lire_itunes(json: &str) -> Vec<Candidat> {
    let Ok(reponse) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(resultats) = reponse.get("results").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    resultats
        .iter()
        .filter_map(|r| {
            let image = r.get("artworkUrl100")?.as_str()?;
            Some(Candidat {
                album: r.get("collectionName")?.as_str()?.to_string(),
                titre: r
                    .get("trackName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                artiste: r
                    .get("artistName")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                // HTTPS seulement : iTunes donne parfois des adresses en http.
                image: en_https(image).replace("100x100bb", "600x600bb"),
            })
        })
        .collect()
}

/// Lit la réponse JSON d'une recherche Deezer, d'albums ou de morceaux
/// (un morceau porte son album). L'image est la plus grande (1000 px).
pub fn lire_deezer(json: &str) -> Vec<Candidat> {
    let Ok(reponse) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(resultats) = reponse.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let texte = |v: &serde_json::Value, cle: &str| v.get(cle)?.as_str().map(str::to_string);
    resultats
        .iter()
        .filter_map(|r| {
            let artiste = r
                .get("artist")
                .and_then(|a| texte(a, "name"))
                .unwrap_or_default();
            let (album, titre) = match r.get("album") {
                Some(album) => (album, texte(r, "title")),
                None => (r, None),
            };
            Some(Candidat {
                album: texte(album, "title")?,
                titre,
                artiste,
                image: en_https(&texte(album, "cover_xl")?),
            })
        })
        .collect()
}

/// La même adresse, en https si elle était en http.
fn en_https(adresse: &str) -> String {
    match adresse.strip_prefix("http://") {
        Some(reste) => format!("https://{reste}"),
        None => adresse.to_string(),
    }
}

/// Le nom de l'album sans « (CD1) », « [Special Edition] »… pour la
/// recherche : « War Eternal (CD1) » n'existe sur aucun site.
fn nom_a_chercher(album: &str) -> String {
    let mut texte = String::new();
    let mut profondeur = 0;
    for c in album.chars() {
        match c {
            '(' | '[' => profondeur += 1,
            ')' | ']' => profondeur = (profondeur - 1).max(0),
            _ if profondeur == 0 => texte.push(c),
            _ => {}
        }
    }
    let texte = texte.split_whitespace().collect::<Vec<_>>().join(" ");
    if texte.is_empty() {
        album.trim().to_string()
    } else {
        texte
    }
}

/// Le nom de l'album sans « (Deluxe Edition) », « [Remastered] »… pour
/// comparer : les éditions d'un même album ont la même pochette, à peu près.
fn sans_edition(album: &str) -> String {
    let texte = normaliser(&nom_a_chercher(album));
    if texte.is_empty() {
        normaliser(album)
    } else {
        texte
    }
}

/// À quel point la réponse ressemble à l'album cherché : le nom, et
/// l'artiste (s'il est connu). Rien si ce n'est pas le même album. Pour un
/// morceau sans album, on compare le titre du morceau.
pub fn noter(candidat: &Candidat, album: &AlbumACompleter) -> Option<f64> {
    let (trouve, cherche) = match &album.morceau {
        Some(morceau) => (candidat.titre.as_deref()?, morceau.as_str()),
        None => (candidat.album.as_str(), album.album.as_str()),
    };
    let titre = ressemblance(&normaliser(trouve), &normaliser(cherche))
        // Une autre édition compte un peu moins que le nom exact.
        .max(0.95 * ressemblance(&sans_edition(trouve), &sans_edition(cherche)));
    if titre < TITRE_MIN {
        return None;
    }
    let artiste = match &album.artiste {
        Some(artiste) => {
            let (cherche, trouve) = (normaliser(artiste), normaliser(&candidat.artiste));
            if !cherche.is_empty() && (trouve.contains(&cherche) || cherche.contains(&trouve)) {
                1.0
            } else {
                ressemblance(&trouve, &cherche)
            }
        }
        None => 0.5,
    };
    (artiste >= ARTISTE_MIN || album.artiste.is_none()).then_some(0.7 * titre + 0.3 * artiste)
}

/// Les bonnes réponses, de la meilleure à la moins bonne, sans doublons.
pub fn garder_les_bonnes(candidats: Vec<Candidat>, album: &AlbumACompleter) -> Vec<Candidat> {
    let mut notes: Vec<(f64, Candidat)> = Vec::new();
    for candidat in candidats {
        if notes.iter().any(|(_, c)| c.image == candidat.image) {
            continue;
        }
        if let Some(note) = noter(&candidat, album) {
            notes.push((note, candidat));
        }
    }
    // Tri stable : à égalité, l'ordre de la source (sa pertinence) reste.
    notes.sort_by(|a, b| b.0.total_cmp(&a.0));
    notes.into_iter().map(|(_, c)| c).collect()
}

/// Met entre guillemets pour la recherche MusicBrainz.
fn entre_guillemets(texte: &str) -> String {
    format!("\"{}\"", texte.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Les recherches à faire, dans l'ordre : MusicBrainz, Deezer, puis
/// iTunes. Un morceau sans album : Deezer puis iTunes, par son titre.
pub fn recherches_a_faire(album: &AlbumACompleter) -> Vec<(Source, Vec<(&'static str, String)>)> {
    if let Some(morceau) = &album.morceau {
        let termes = match &album.artiste {
            Some(artiste) => format!("{artiste} {morceau}"),
            None => morceau.clone(),
        };
        return vec![
            (
                Source::DeezerMorceaux,
                vec![("q", termes.clone()), ("limit", "10".to_string())],
            ),
            (
                Source::Itunes,
                vec![
                    ("term", termes),
                    ("entity", "song".to_string()),
                    ("limit", "10".to_string()),
                ],
            ),
        ];
    }
    let nom = nom_a_chercher(&album.album);
    let mut requete = format!("releasegroup:{}", entre_guillemets(&nom));
    if let Some(artiste) = &album.artiste {
        requete += &format!(" AND artist:{}", entre_guillemets(artiste));
    }
    let termes = match &album.artiste {
        Some(artiste) => format!("{artiste} {nom}"),
        None => nom,
    };
    vec![
        (
            Source::MusicBrainz,
            vec![
                ("query", requete),
                ("fmt", "json".to_string()),
                ("limit", "10".to_string()),
            ],
        ),
        (
            Source::DeezerAlbums,
            vec![("q", termes.clone()), ("limit", "10".to_string())],
        ),
        (
            Source::Itunes,
            vec![
                ("term", termes),
                ("entity", "album".to_string()),
                ("limit", "10".to_string()),
            ],
        ),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    MusicBrainz,
    DeezerAlbums,
    DeezerMorceaux,
    Itunes,
}

impl Source {
    fn adresse(self) -> &'static str {
        match self {
            Source::MusicBrainz => MUSICBRAINZ,
            Source::DeezerAlbums => DEEZER_ALBUMS,
            Source::DeezerMorceaux => DEEZER_MORCEAUX,
            Source::Itunes => ITUNES,
        }
    }

    fn lire(self, json: &str) -> Vec<Candidat> {
        match self {
            Source::MusicBrainz => lire_musicbrainz(json),
            Source::DeezerAlbums | Source::DeezerMorceaux => lire_deezer(json),
            Source::Itunes => lire_itunes(json),
        }
    }

    fn nom(self) -> &'static str {
        match self {
            Source::MusicBrainz => "Cover Art Archive",
            Source::DeezerAlbums | Source::DeezerMorceaux => "Deezer",
            Source::Itunes => "iTunes",
        }
    }
}

/// Pourquoi une recherche s'est arrêtée avant la fin.
#[derive(Debug, PartialEq)]
pub enum Interruption {
    /// On a cliqué sur « Arrêter ».
    Arretee,
    /// Le site n'a pas répondu, ou a refusé.
    Reseau(Echec),
}

/// Une image trouvée et d'où elle vient (« iTunes : Album — Artiste »).
#[derive(Debug, PartialEq)]
pub struct Trouvee {
    pub image: Vec<u8>,
    pub source: String,
}

/// Cherche la pochette d'un album en ligne. `demander` télécharge une
/// adresse avec ses paramètres (dans les tests, fait semblant) : rien si la
/// page n'existe pas. `arreter` dit si on a cliqué sur « Arrêter ».
pub fn chercher(
    album: &AlbumACompleter,
    mut demander: impl FnMut(&str, &[(&str, String)]) -> Result<Option<Vec<u8>>, Echec>,
    arreter: impl Fn() -> bool,
    pause: Duration,
) -> Result<Option<Trouvee>, Interruption> {
    for (source, parametres) in recherches_a_faire(album) {
        if arreter() {
            return Err(Interruption::Arretee);
        }
        let reponse = demander(source.adresse(), &parametres).map_err(Interruption::Reseau)?;
        if source == Source::MusicBrainz {
            std::thread::sleep(pause);
        }
        let json = String::from_utf8_lossy(&reponse.unwrap_or_default()).into_owned();
        let bonnes = garder_les_bonnes(source.lire(&json), album);
        for candidat in bonnes.into_iter().take(IMAGES_ESSAYEES) {
            if arreter() {
                return Err(Interruption::Arretee);
            }
            // Pas d'image pour cet album (fréquent) : on essaie le suivant.
            let Some(image) = demander(&candidat.image, &[]).map_err(Interruption::Reseau)? else {
                continue;
            };
            if est_une_image(&image) {
                return Ok(Some(Trouvee {
                    image,
                    source: format!(
                        "{} : {} — {}",
                        source.nom(),
                        candidat.album,
                        candidat.artiste
                    ),
                }));
            }
        }
    }
    Ok(None)
}

/// Un JPEG ou un PNG lisible (ce que `ecriture_tags` accepte).
fn est_une_image(octets: &[u8]) -> bool {
    matches!(
        image::guess_format(octets),
        Ok(image::ImageFormat::Jpeg | image::ImageFormat::Png)
    ) && image::load_from_memory(octets).is_ok()
}

// ---------------------------------------------------------------- en arrière-plan

/// Où en est la recherche des pochettes.
#[derive(Debug, Default)]
pub struct Progression {
    /// On lit encore les tags de la bibliothèque (le total n'est pas connu).
    pub preparation: bool,
    /// Albums traités, sur le nombre d'albums à compléter.
    pub faits: usize,
    pub total: usize,
    /// L'album qu'on cherche en ce moment.
    pub en_cours: Option<String>,
    /// Les albums dont la pochette a été trouvée en ligne.
    pub trouvees: usize,
    /// Les albums dont la pochette a été recopiée d'un autre morceau.
    pub recopiees: usize,
    /// Les albums dont tous les morceaux avaient déjà une pochette.
    pub completes: usize,
    pub introuvables: Vec<String>,
    /// Les albums déjà cherchés sans succès
    /// sautés.
    pub deja_cherches: usize,
    /// Le site n'a pas répondu, ou un MP3 n'a pas pu être écrit.
    pub erreurs: Vec<(String, String)>,
    /// Les MP3 qui ont reçu une pochette depuis la dernière fois qu'on a
    /// demandé : si c'est le morceau en cours, sa pochette s'affiche.
    pub nouveaux: Vec<PathBuf>,
    /// Pourquoi on s'est arrêté avant la fin, s'il y a lieu.
    pub interruption: Option<String>,
    pub fini: bool,
}

/// La recherche des pochettes, qui tourne en arrière-plan.
pub struct RecherchePochettes {
    progression: Arc<Mutex<Progression>>,
    arreter: Arc<AtomicBool>,
}

impl RecherchePochettes {
    /// Cherche les pochettes qui manquent parmi ces morceaux, album par
    /// album, en arrière-plan.
    pub fn lancer(ctx: &egui::Context, morceaux: Vec<PathBuf>) -> Self {
        let progression = Arc::new(Mutex::new(Progression {
            preparation: true,
            ..Progression::default()
        }));
        let arreter = Arc::new(AtomicBool::new(false));
        let (pour_le_travail, stop, ctx) =
            (Arc::clone(&progression), Arc::clone(&arreter), ctx.clone());
        std::thread::spawn(move || {
            let agent = reseau::agent(Duration::from_secs(30));
            completer(
                &morceaux,
                &pour_le_travail,
                &stop,
                &mut Introuvables::du_lecteur(introuvables::POCHETTES),
                |adresse, p| reseau::telecharger(&agent, adresse, p, &stop),
                PAUSE_ENTRE_RECHERCHES,
                || ctx.request_repaint(),
            );
        });
        Self {
            progression,
            arreter,
        }
    }

    pub fn progression(&self) -> MutexGuard<'_, Progression> {
        // Si le travail avait planté en tenant le verrou, on affiche quand même.
        self.progression.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Demande l'arrêt : l'album en cours est abandonné.
    pub fn arreter(&self) {
        self.arreter.store(true, Ordering::Relaxed);
    }

    pub fn arret_demande(&self) -> bool {
        self.arreter.load(Ordering::Relaxed)
    }
}

/// Le travail lui-même : on lit les tags de tous les MP3, on les range par
/// album, puis on complète chaque album, l'un après l'autre.
fn completer(
    morceaux: &[PathBuf],
    progression: &Mutex<Progression>,
    arreter: &AtomicBool,
    memoire: &mut Introuvables,
    mut demander: impl FnMut(&str, &[(&str, String)]) -> Result<Option<Vec<u8>>, Echec>,
    pause: Duration,
    redessiner: impl Fn(),
) {
    let etat = || progression.lock().unwrap_or_else(|e| e.into_inner());
    let arret = || arreter.load(Ordering::Relaxed);
    let fin = |interruption: Option<&str>| {
        let mut etat = etat();
        etat.interruption = interruption.map(str::to_string);
        etat.en_cours = None;
        etat.preparation = false;
        etat.fini = true;
        drop(etat);
        redessiner();
    };

    // Les pochettes ne s'écrivent que dans les MP3.
    let mut lus = Vec::new();
    for chemin in morceaux {
        if arret() {
            return fin(Some("Arrêté."));
        }
        if !chemin
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
        {
            continue;
        }
        lus.push(MorceauLu {
            chemin: chemin.clone(),
            tags: tags::lire(chemin),
            a_une_pochette: tags::lire_pochette_brute(chemin).is_some(),
        });
    }
    let (albums, completes) = albums_a_completer(lus);
    // Déjà cherchés sans succès sautés, sauf si
    // un morceau de l'album a maintenant une pochette à recopier.
    let (albums, sautes): (Vec<_>, Vec<_>) = albums.into_iter().partition(|album| {
        album.avec_pochette.is_some() || !memoire.deja_cherche(&album.libelle())
    });
    {
        let mut etat = etat();
        etat.preparation = false;
        etat.total = albums.len();
        etat.completes = completes;
        etat.deja_cherches = sautes.len();
    }
    redessiner();

    let mut echecs_de_suite = 0;
    for album in &albums {
        if arret() {
            return fin(Some("Arrêté."));
        }
        etat().en_cours = Some(album.libelle());
        redessiner();

        // 1. Un autre morceau de l'album a déjà sa pochette : on la recopie.
        let recopiee = album
            .avec_pochette
            .as_deref()
            .and_then(tags::lire_pochette_brute)
            .filter(|image| est_une_image(image));
        let trouvee = match recopiee {
            Some(image) => Ok(Some((image, true))),
            // 2. et 3. On la cherche en ligne.
            None => {
                chercher(album, &mut demander, arret, pause).map(|t| t.map(|t| (t.image, false)))
            }
        };

        let mut etat = etat();
        match trouvee {
            Ok(Some((image, recopiee))) => {
                echecs_de_suite = 0;
                let mut ecrits = 0;
                for chemin in &album.sans_pochette {
                    let modifications = Modifications {
                        textes: Vec::new(),
                        pochette: Pochette::Remplacer(image.clone()),
                    };
                    // Une pochette ajoutée entre-temps n'est pas remplacée.
                    if tags::lire_pochette_brute(chemin).is_some() {
                        continue;
                    }
                    match ecriture_tags::enregistrer(chemin, &modifications) {
                        Ok(()) => {
                            ecrits += 1;
                            etat.nouveaux.push(chemin.clone());
                        }
                        Err(erreur) => etat
                            .erreurs
                            .push((crate::liste::nom_du_fichier(chemin), erreur.to_string())),
                    }
                }
                if ecrits > 0 {
                    if recopiee {
                        etat.recopiees += 1;
                    } else {
                        etat.trouvees += 1;
                    }
                }
            }
            Ok(None) => {
                echecs_de_suite = 0;
                etat.introuvables.push(album.libelle());
                memoire.noter(&album.libelle(), introuvables::maintenant());
            }
            Err(Interruption::Arretee) => {
                drop(etat);
                return fin(Some("Arrêté."));
            }
            Err(Interruption::Reseau(echec)) => {
                etat.erreurs.push((album.libelle(), echec.to_string()));
                echecs_de_suite += 1;
                if echecs_de_suite >= ECHECS_DE_SUITE_MAX {
                    etat.faits += 1;
                    drop(etat);
                    return fin(Some(match echec {
                        Echec::SiteIndisponible(_) => {
                            "Les sites de pochettes sont indisponibles pour le moment \
                             (surchargés ou en maintenance) : réessaie plus tard."
                        }
                        Echec::SansReponse(_) => {
                            "Les sites de pochettes ne répondent pas : la connexion Internet \
                             est peut-être coupée."
                        }
                        Echec::NonSecurise(_) => {
                            "Un site de pochettes a donné une adresse non sécurisée (http)."
                        }
                    }));
                }
            }
        }
        etat.faits += 1;
    }
    fin(None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::tests::ecrire_mp3_avec;

    fn tags(album: Option<&str>, artiste: Option<&str>, artiste_album: Option<&str>) -> Tags {
        let mut tags = Tags::default();
        tags.album = album.map(str::to_string);
        tags.artiste = artiste.map(str::to_string);
        tags.artiste_album = artiste_album.map(str::to_string);
        tags
    }

    fn lu(chemin: &str, tags: Tags, a_une_pochette: bool) -> MorceauLu {
        MorceauLu {
            chemin: PathBuf::from(chemin),
            tags,
            a_une_pochette,
        }
    }

    fn album(nom: &str, artiste: Option<&str>) -> AlbumACompleter {
        AlbumACompleter {
            album: nom.to_string(),
            morceau: None,
            artiste: artiste.map(str::to_string),
            sans_pochette: Vec::new(),
            avec_pochette: None,
        }
    }

    fn candidat(album: &str, artiste: &str, image: &str) -> Candidat {
        Candidat {
            album: album.to_string(),
            titre: None,
            artiste: artiste.to_string(),
            image: image.to_string(),
        }
    }

    fn jpeg() -> Vec<u8> {
        let image = image::RgbImage::from_pixel(5, 5, image::Rgb([0, 255, 255]));
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        jpeg.into_inner()
    }

    #[test]
    fn range_par_album_comme_la_vue_albums() {
        let dp = Some("Daft Punk");
        let (albums, complets) = albums_a_completer(vec![
            lu(
                "M/DP/Discovery/1.mp3",
                tags(Some("Discovery"), dp, None),
                false,
            ),
            lu(
                "M/DP/Discovery/2.mp3",
                tags(Some("discovery "), dp, None),
                true,
            ),
            lu(
                "M/DP/Discovery/3.mp3",
                tags(Some("Discovery"), dp, None),
                false,
            ),
            lu(
                "M/DP/Homework/1.mp3",
                tags(Some("Homework"), dp, None),
                true,
            ),
            lu("M/Divers/sans album.mp3", tags(None, dp, None), false),
            // Une compilation : plusieurs artistes, un seul dossier.
            lu(
                "M/Hits/a.mp3",
                tags(Some("Hits 2000"), Some("A"), None),
                false,
            ),
            lu(
                "M/Hits/b.mp3",
                tags(Some("Hits 2000"), Some("B"), None),
                false,
            ),
            // Même artiste de l'album, dossiers différents : un seul album.
            lu(
                "M/X/cd1/a.mp3",
                tags(Some("Live"), Some("X feat. Y"), Some("X")),
                false,
            ),
            lu(
                "M/X/cd2/b.mp3",
                tags(Some("Live"), Some("X"), Some("X")),
                false,
            ),
        ]);
        assert_eq!(complets, 1); // Homework
        assert_eq!(albums.len(), 4);
        assert_eq!(albums[0].album, "Discovery");
        assert_eq!(albums[0].artiste.as_deref(), Some("Daft Punk"));
        assert_eq!(
            albums[0].sans_pochette,
            [
                PathBuf::from("M/DP/Discovery/1.mp3"),
                PathBuf::from("M/DP/Discovery/3.mp3")
            ]
        );
        assert_eq!(
            albums[0].avec_pochette,
            Some(PathBuf::from("M/DP/Discovery/2.mp3"))
        );
        assert_eq!(albums[1].libelle(), "Hits 2000");
        assert_eq!(albums[1].sans_pochette.len(), 2);
        assert_eq!(albums[2].libelle(), "Live — X");
        assert_eq!(albums[2].sans_pochette.len(), 2);
        // Le morceau sans album vient après, cherché par son titre.
        assert_eq!(albums[3].morceau.as_deref(), Some("sans album"));
    }

    #[test]
    fn lit_la_reponse_de_musicbrainz() {
        let json = r#"{"release-groups": [
            {"id": "abc-123", "title": "Discovery",
             "artist-credit": [{"name": "Daft Punk", "joinphrase": " & "}, {"name": "Romanthony"}]},
            {"title": "sans id"}
        ]}"#;
        assert_eq!(
            lire_musicbrainz(json),
            [candidat(
                "Discovery",
                "Daft Punk & Romanthony",
                "https://coverartarchive.org/release-group/abc-123/front-500"
            )]
        );
        assert!(lire_musicbrainz("pas du json").is_empty());
    }

    #[test]
    fn lit_la_reponse_d_itunes_en_600_pixels() {
        let json = r#"{"resultCount": 1, "results": [
            {"collectionName": "Discovery", "artistName": "Daft Punk",
             "artworkUrl100": "https://is1.mzstatic.com/image/thumb/x/100x100bb.jpg"},
            {"collectionName": "Sans image", "artistName": "Daft Punk"}
        ]}"#;
        assert_eq!(
            lire_itunes(json),
            [candidat(
                "Discovery",
                "Daft Punk",
                "https://is1.mzstatic.com/image/thumb/x/600x600bb.jpg"
            )]
        );
    }

    #[test]
    fn une_image_itunes_en_http_passe_en_https() {
        let json = r#"{"results": [{"collectionName": "Discovery", "artistName": "Daft Punk",
             "artworkUrl100": "http://is1.mzstatic.com/image/thumb/x/100x100bb.jpg"}]}"#;
        assert_eq!(
            lire_itunes(json)[0].image,
            "https://is1.mzstatic.com/image/thumb/x/600x600bb.jpg"
        );
    }

    /// Exemples de Luce : « War Eternal (CD1) » et « Black Dog Barking
    /// [Special Edition - CD1] » ne sont trouvés nulle part tels quels.
    #[test]
    fn cherche_l_album_sans_cd1_ni_edition() {
        let recherches = recherches_a_faire(&album("War Eternal (CD1)", Some("Arch Enemy")));
        let sources: Vec<_> = recherches.iter().map(|(s, _)| *s).collect();
        assert_eq!(
            sources,
            [Source::MusicBrainz, Source::DeezerAlbums, Source::Itunes]
        );
        assert_eq!(
            recherches[0].1[0].1,
            r#"releasegroup:"War Eternal" AND artist:"Arch Enemy""#
        );
        assert_eq!(recherches[1].1[0].1, "Arch Enemy War Eternal");
        assert_eq!(
            nom_a_chercher("Black Dog Barking [Special Edition - CD1]"),
            "Black Dog Barking"
        );
        // La réponse « War Eternal » est bien le même album.
        assert!(noter(
            &candidat("War Eternal", "Arch Enemy", "x"),
            &album("War Eternal (CD1)", Some("Arch Enemy"))
        )
        .is_some());
    }

    #[test]
    fn lit_la_reponse_de_deezer_albums_et_morceaux() {
        let albums = r#"{"data": [{"title": "War Eternal", "artist": {"name": "Arch Enemy"},
            "cover_xl": "http://cdn-images.dzcdn.net/images/cover/abc/1000x1000-000000-80-0-0.jpg"}]}"#;
        assert_eq!(
            lire_deezer(albums),
            [candidat(
                "War Eternal",
                "Arch Enemy",
                "https://cdn-images.dzcdn.net/images/cover/abc/1000x1000-000000-80-0-0.jpg"
            )]
        );
        let morceaux = r#"{"data": [{"title": "Tequila Coma", "artist": {"name": "yeule"},
            "album": {"title": "Evangelic Girl is a Gun", "cover_xl": "https://d/1000.jpg"}}]}"#;
        let lus = lire_deezer(morceaux);
        assert_eq!(lus[0].album, "Evangelic Girl is a Gun");
        assert_eq!(lus[0].titre.as_deref(), Some("Tequila Coma"));
        assert!(lire_deezer(r#"{"error": {"code": 4}}"#).is_empty());
    }

    #[test]
    fn un_morceau_sans_album_est_cherche_par_son_titre() {
        let (albums, _) = albums_a_completer(vec![
            lu(
                "M/yeule/yeule - 'Tequila Coma' (Official Video).mp3",
                tags(None, None, None),
                false,
            ),
            // Déjà une pochette : rien à faire.
            lu("M/X/X - Y.mp3", tags(None, None, None), true),
        ]);
        assert_eq!(albums.len(), 1);
        let seul = &albums[0];
        assert_eq!(seul.morceau.as_deref(), Some("Tequila Coma"));
        assert_eq!(seul.artiste.as_deref(), Some("yeule"));
        assert_eq!(seul.libelle(), "Tequila Coma — yeule");

        let recherches = recherches_a_faire(seul);
        assert_eq!(recherches[0].0, Source::DeezerMorceaux);
        assert_eq!(recherches[0].1[0].1, "yeule Tequila Coma");
        assert_eq!(recherches[1].0, Source::Itunes);
        assert!(recherches[1].1.contains(&("entity", "song".to_string())));

        // On compare le titre du morceau, pas le nom de l'album.
        let mut bon = candidat("Evangelic Girl is a Gun", "yeule", "a");
        bon.titre = Some("Tequila Coma".to_string());
        let mut autre = candidat("Tequila Coma", "yeule", "b");
        autre.titre = Some("Bites on My Neck".to_string());
        assert_eq!(garder_les_bonnes(vec![autre, bon.clone()], seul), [bon]);
    }

    #[test]
    fn garde_seulement_le_bon_album() {
        let cherche = album("Discovery", Some("Daft Punk"));
        let bonnes = garder_les_bonnes(
            vec![
                candidat("Homework", "Daft Punk", "1"),
                candidat("Discovery", "Autre groupe", "2"),
                candidat("Discovery (Deluxe Edition)", "Daft Punk", "3"),
                candidat("Discovery", "Daft Punk", "4"),
                candidat("Discovery", "Daft Punk", "4"),
            ],
            &cherche,
        );
        let images: Vec<&str> = bonnes.iter().map(|c| c.image.as_str()).collect();
        assert_eq!(images, ["4", "3"]);
        // Compilation : l'artiste ne compte pas.
        let compilation = album("Hits 2000", None);
        assert_eq!(
            garder_les_bonnes(
                vec![candidat("Hits 2000", "Various Artists", "5")],
                &compilation
            )
            .len(),
            1
        );
    }

    #[test]
    fn recherches_musicbrainz_puis_deezer_puis_itunes() {
        let recherches = recherches_a_faire(&album("Le \"Best\" of", Some("Moi")));
        assert_eq!(recherches[0].0, Source::MusicBrainz);
        assert_eq!(
            recherches[0].1[0],
            (
                "query",
                r#"releasegroup:"Le \"Best\" of" AND artist:"Moi""#.to_string()
            )
        );
        assert_eq!(recherches[1].0, Source::DeezerAlbums);
        assert_eq!(recherches[2].0, Source::Itunes);
        assert_eq!(
            recherches[2].1[0],
            ("term", "Moi Le \"Best\" of".to_string())
        );
    }

    #[test]
    fn passe_a_itunes_si_ni_le_cover_art_archive_ni_deezer_n_ont_d_image() {
        let cherche = album("Discovery", Some("Daft Punk"));
        let mut demandes = Vec::new();
        let trouvee = chercher(
            &cherche,
            |adresse, _| {
                demandes.push(adresse.to_string());
                Ok(match adresse {
                    MUSICBRAINZ => Some(
                        br#"{"release-groups": [{"id": "x", "title": "Discovery",
                            "artist-credit": [{"name": "Daft Punk"}]}]}"#
                            .to_vec(),
                    ),
                    ITUNES => Some(
                        br#"{"results": [{"collectionName": "Discovery", "artistName": "Daft Punk",
                            "artworkUrl100": "https://itunes/100x100bb.jpg"}]}"#
                            .to_vec(),
                    ),
                    "https://itunes/600x600bb.jpg" => Some(jpeg()),
                    _ => None, // le Cover Art Archive n'a pas d'image
                })
            },
            || false,
            Duration::ZERO,
        )
        .unwrap()
        .unwrap();
        assert_eq!(trouvee.image, jpeg());
        assert_eq!(trouvee.source, "iTunes : Discovery — Daft Punk");
        assert_eq!(
            demandes,
            [
                MUSICBRAINZ,
                "https://coverartarchive.org/release-group/x/front-500",
                DEEZER_ALBUMS,
                ITUNES,
                "https://itunes/600x600bb.jpg"
            ]
        );
        // Bouton « Arrêter » : on n'interroge plus personne.
        assert_eq!(
            chercher(
                &cherche,
                |_, _| panic!("ne doit pas chercher"),
                || true,
                Duration::ZERO
            ),
            Err(Interruption::Arretee)
        );
    }

    fn dossier_de_test(nom: &str) -> PathBuf {
        let dossier = std::env::temp_dir().join(format!("lecteur_mp3_pochettes_{nom}"));
        let _ = std::fs::remove_dir_all(&dossier);
        std::fs::create_dir_all(&dossier).unwrap();
        dossier
    }

    /// Un MP3 de test, rangé dans `dossier`.
    fn mp3(dossier: &Path, nom: &str, album: &str, pochette: bool) -> PathBuf {
        let cree = ecrire_mp3_avec(
            &format!("pochettes {nom}"),
            &[("TALB", album), ("TPE1", "Daft Punk")],
            pochette,
            20,
        );
        let chemin = dossier.join(nom);
        std::fs::rename(cree, &chemin).unwrap();
        chemin
    }

    #[test]
    fn recopie_cherche_et_ne_remplace_jamais() {
        let dossier = dossier_de_test("completer");
        let deja = mp3(&dossier, "a.mp3", "Discovery", true);
        let a_recopier = mp3(&dossier, "b.mp3", "Discovery", false);
        let en_ligne = mp3(&dossier, "c.mp3", "Homework", false);
        let introuvable = mp3(&dossier, "d.mp3", "Rien du tout", false);
        let pochette_d_origine = tags::lire_pochette_brute(&deja).unwrap();

        let progression = Mutex::new(Progression::default());
        completer(
            &[
                deja.clone(),
                a_recopier.clone(),
                en_ligne.clone(),
                introuvable.clone(),
            ],
            &progression,
            &AtomicBool::new(false),
            &mut Introuvables::en_memoire(),
            |adresse, parametres| {
                let cherche_homework = parametres.iter().any(|(_, v)| v.contains("Homework"));
                Ok(match adresse {
                    MUSICBRAINZ if cherche_homework => Some(
                        br#"{"release-groups": [{"id": "h", "title": "Homework",
                            "artist-credit": [{"name": "Daft Punk"}]}]}"#
                            .to_vec(),
                    ),
                    "https://coverartarchive.org/release-group/h/front-500" => Some(jpeg()),
                    _ => None,
                })
            },
            Duration::ZERO,
            || {},
        );
        let p = progression.into_inner().unwrap();
        // La pochette existante n'a pas bougé, et a été recopiée.
        assert_eq!(
            tags::lire_pochette_brute(&deja).unwrap(),
            pochette_d_origine
        );
        assert_eq!(
            tags::lire_pochette_brute(&a_recopier).unwrap(),
            pochette_d_origine
        );
        assert_eq!(tags::lire_pochette_brute(&en_ligne).unwrap(), jpeg());
        assert!(tags::lire_pochette_brute(&introuvable).is_none());
        assert_eq!((p.faits, p.total, p.trouvees, p.recopiees), (3, 3, 1, 1));
        assert_eq!(p.introuvables, ["Rien du tout — Daft Punk"]);
        assert_eq!(p.nouveaux, [a_recopier, en_ligne]);
        assert!(p.fini && p.interruption.is_none() && p.erreurs.is_empty());
    }

    #[test]
    fn un_album_introuvable_n_est_pas_recherche_la_fois_suivante() {
        let dossier = dossier_de_test("memoire");
        let morceaux = [mp3(&dossier, "memoire.mp3", "Rien du tout", false)];
        let mut memoire = Introuvables::en_memoire();
        let appels = std::cell::Cell::new(0);
        let mut resumes = Vec::new();
        for _ in 0..2 {
            let progression = Mutex::new(Progression::default());
            completer(
                &morceaux,
                &progression,
                &AtomicBool::new(false),
                &mut memoire,
                |_, _| {
                    appels.set(appels.get() + 1);
                    Ok(None)
                },
                Duration::ZERO,
                || {},
            );
            let p = progression.into_inner().unwrap();
            resumes.push((p.introuvables.len(), p.deja_cherches, p.total));
        }
        assert_eq!(resumes, [(1, 0, 1), (0, 1, 0)]);
        // Les recherches du premier passage seulement.
        assert_eq!(appels.get(), 3);
    }

    #[test]
    fn s_arrete_si_internet_ne_repond_pas() {
        let dossier = dossier_de_test("reseau");
        let morceaux: Vec<PathBuf> = (1..=5)
            .map(|n| mp3(&dossier, &format!("{n}.mp3"), &format!("Album {n}"), false))
            .collect();
        let progression = Mutex::new(Progression::default());
        let appels = std::cell::Cell::new(0);
        completer(
            &morceaux,
            &progression,
            &AtomicBool::new(false),
            &mut Introuvables::en_memoire(),
            |_, _| {
                appels.set(appels.get() + 1);
                Err(Echec::SansReponse("pas de connexion".to_string()))
            },
            Duration::ZERO,
            || {},
        );
        let p = progression.into_inner().unwrap();
        assert_eq!(appels.get(), ECHECS_DE_SUITE_MAX);
        assert_eq!(p.erreurs.len(), ECHECS_DE_SUITE_MAX);
        assert!(p.interruption.unwrap().contains("connexion Internet") && p.fini);
    }

    #[test]
    fn un_503_dit_que_les_sites_sont_indisponibles() {
        let dossier = dossier_de_test("503");
        // Des noms à part : `mp3` passe par un fichier temporaire à ce nom,
        // partagé avec les tests qui tournent en même temps.
        let morceaux: Vec<PathBuf> = (1..=5)
            .map(|n| {
                mp3(
                    &dossier,
                    &format!("503-{n}.mp3"),
                    &format!("Album {n}"),
                    false,
                )
            })
            .collect();
        let progression = Mutex::new(Progression::default());
        completer(
            &morceaux,
            &progression,
            &AtomicBool::new(false),
            &mut Introuvables::en_memoire(),
            |_, _| Err(Echec::SiteIndisponible("http status: 503".to_string())),
            Duration::ZERO,
            || {},
        );
        let p = progression.into_inner().unwrap();
        let message = p.interruption.unwrap();
        assert!(message.contains("indisponibles"), "{message}");
        assert!(!message.contains("connexion"), "{message}");
    }
}
