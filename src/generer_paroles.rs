//! Le bouton « Paroles » de la bibliothèque : pour chaque MP3 qui n'a pas
//! encore de .lrc, on cherche ses paroles synchronisées sur LRCLIB
//! (https://lrclib.net, gratuit, sans compte) et on les range dans notre
//! dossier des paroles (voir paroles.rs), pas dans le dossier Musique.
//!
//! C'est le script generer_lrc.py de Luce, repris tel quel en Rust :
//! - on devine plusieurs titres et artistes possibles (nom du fichier
//!   nettoyé des « (Official Video) », tags, dossiers) ;
//! - on interroge LRCLIB avec plusieurs combinaisons ;
//! - on note chaque réponse (titre, artiste, durée) et on garde la
//!   meilleure, seulement si elle ressemble assez au morceau.
//!
//! Un .lrc déjà présent n'est jamais remplacé.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use eframe::egui;
use regex::Regex;

use crate::paroles::Rangement;
use crate::tags::{self, Tags};

const API: &str = "https://lrclib.net/api";
/// LRCLIB demande que chaque logiciel se présente.
const AGENT: &str = concat!(
    "lecteur_mp3/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/lepapyfoufurieux/lecteur_mp3)"
);
/// Une petite pause entre deux recherches, pour ne pas bombarder LRCLIB.
const PAUSE_ENTRE_RECHERCHES: Duration = Duration::from_millis(200);
/// Au-delà de cette note, la réponse est assez bonne : on arrête de chercher.
const NOTE_SUFFISANTE: f64 = 0.93;
/// Après autant de morceaux de suite sans réussir à joindre LRCLIB, on
/// s'arrête : Internet est sans doute coupé.
const ECHECS_DE_SUITE_MAX: usize = 3;

/// Mentions YouTube ou d'édition à retirer du titre.
const MENTIONS: &str = r"(official|offical|audio|video|vid[eé]o|visuali[sz]er|lyrics?|lyric video|clip|music video|mv|hd|hq|4k|remaster(ed)?|explicit|topic|full album|live)";

// ---------------------------------------------------------------- comparer

/// Minuscules, sans accents, sans ponctuation : pour comparer deux titres.
pub fn normaliser(texte: &str) -> String {
    let texte: String = texte.chars().map(sans_accent).collect();
    let texte = texte.to_lowercase().replace('&', " and ");
    let texte: String = texte
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    texte.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// La lettre sans son accent (é → e). Les autres caractères restent tels quels.
fn sans_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => 'A',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => 'C',
        'ď' => 'd',
        'Ď' => 'D',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => 'E',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => 'G',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
        'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī' | 'Ĭ' | 'Į' | 'İ' => 'I',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'Ñ' | 'Ń' | 'Ņ' | 'Ň' => 'N',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' => 'o',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ō' | 'Ŏ' | 'Ő' => 'O',
        'ŕ' | 'ř' => 'r',
        'Ŕ' | 'Ř' => 'R',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'Ś' | 'Ŝ' | 'Ş' | 'Š' => 'S',
        'ţ' | 'ť' => 't',
        'Ţ' | 'Ť' => 'T',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => 'U',
        'ý' | 'ÿ' => 'y',
        'Ý' | 'Ÿ' => 'Y',
        'ź' | 'ż' | 'ž' => 'z',
        'Ź' | 'Ż' | 'Ž' => 'Z',
        _ => c,
    }
}

/// Retire les guillemets et les espaces autour.
fn sans_guillemets(texte: &str) -> &str {
    texte
        .trim()
        .trim_matches(|c| "'\"‘’“”«»`´ ".contains(c))
        .trim()
}

fn regex(motif: &str) -> Regex {
    Regex::new(motif).expect("expression régulière valide")
}

/// Retire numéro de piste, mentions « (Official Audio) », « [HD] »,
/// « | … », « - Official Video ».
pub fn nettoyer(texte: &str) -> String {
    static NUMERO: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s*\d{1,3}\s*[.\-_)]\s*")); // 01. / 01 - / 01)
    static NUMERO_ESPACE: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s*\d{1,3}\s+(\D)")); // « 01 titre »
    static ENTRE_PARENTHESES: LazyLock<Regex> = LazyLock::new(|| {
        regex(&format!(
            r"(?i)[\(\[\{{][^\)\]\}}]*{MENTIONS}[^\)\]\}}]*[\)\]\}}]"
        ))
    });
    static BARRE: LazyLock<Regex> = LazyLock::new(|| regex(r"\s[|｜]\s.*$")); // « titre | blabla »
    static TIRET: LazyLock<Regex> =
        LazyLock::new(|| regex(&format!(r"(?i)\s+-\s+{MENTIONS}\b.*$")));

    let texte = texte.replace('_', " ");
    let texte = NUMERO.replace(&texte, "");
    let texte = NUMERO_ESPACE.replace(&texte, "$1");
    let texte = ENTRE_PARENTHESES.replace_all(&texte, "");
    let texte = BARRE.replace(&texte, "");
    let texte = TIRET.replace(&texte, "");
    let texte = texte.split_whitespace().collect::<Vec<_>>().join(" ");
    texte.trim_matches(|c| c == ' ' || c == '-').to_string()
}

/// Le titre sans « feat. … », « ft. … », « (with …) ».
fn sans_invites(texte: &str) -> String {
    static INVITES: LazyLock<Regex> =
        LazyLock::new(|| regex(r"(?i)\s*[\(\[]?\s*(feat\.?|ft\.?|featuring|with)\s.*$"));
    INVITES.replace(texte, "").trim().to_string()
}

/// La ressemblance de deux textes, de 0 (rien en commun) à 1 (identiques),
/// calculée comme `difflib.SequenceMatcher.ratio()` de Python : on cherche
/// le plus long morceau commun, puis on recommence à sa gauche et à sa droite.
pub fn ressemblance(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut communs = 0;
    let mut a_voir = vec![(0, a.len(), 0, b.len())];
    while let Some((a_debut, a_fin, b_debut, b_fin)) = a_voir.pop() {
        let (i, j, taille) = plus_long_commun(&a[a_debut..a_fin], &b[b_debut..b_fin]);
        if taille == 0 {
            continue;
        }
        let (i, j) = (a_debut + i, b_debut + j);
        communs += taille;
        a_voir.push((a_debut, i, b_debut, j));
        a_voir.push((i + taille, a_fin, j + taille, b_fin));
    }
    2.0 * communs as f64 / (a.len() + b.len()) as f64
}

/// Le plus long morceau commun à `a` et `b` : sa place dans `a`, dans `b`,
/// et sa longueur. À égalité, celui qui commence le plus tôt dans `a`, puis
/// dans `b` (comme Python).
fn plus_long_commun(a: &[char], b: &[char]) -> (usize, usize, usize) {
    let mut meilleur = (0, 0, 0);
    // longueurs[j + 1] : longueur du morceau commun qui finit en a[i] et b[j].
    let mut precedentes = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        let mut longueurs = vec![0; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            if ca == cb {
                let k = precedentes[j] + 1;
                longueurs[j + 1] = k;
                if k > meilleur.2 {
                    meilleur = (i + 1 - k, j + 1 - k, k);
                }
            }
        }
        precedentes = longueurs;
    }
    meilleur
}

// ---------------------------------------------------------------- le MP3

/// Ce qu'on cherche : les artistes et titres possibles, l'album et la durée.
#[derive(Debug, Default, PartialEq)]
pub struct Recherche {
    pub artistes: Vec<String>,
    pub titres: Vec<String>,
    pub album: String,
    /// En secondes, arrondie.
    pub duree: f64,
}

/// Ajoute `valeur` à la liste si elle n'y est pas déjà (à la ponctuation
/// et aux accents près).
fn ajouter(liste: &mut Vec<String>, valeur: &str) {
    let valeur = sans_guillemets(valeur);
    let cle = normaliser(valeur);
    if !cle.is_empty() && !liste.iter().any(|v| normaliser(v) == cle) {
        liste.push(valeur.to_string());
    }
}

/// Devine les artistes et titres possibles d'un MP3, d'après le nom du
/// fichier (souvent « Artiste - Titre (Official Video) »), ses tags et ses
/// dossiers.
pub fn preparer(chemin: &Path, tags: &Tags) -> Recherche {
    static TIRET: LazyLock<Regex> = LazyLock::new(|| regex(r"\s+[-–—]\s+"));
    static GUILLEMETS: LazyLock<Regex> = LazyLock::new(|| regex(r#"['"‘“]([^'"’”]{1,80})['"’”]"#));
    static TOPIC: LazyLock<Regex> = LazyLock::new(|| regex(r"(?i)\s*-\s*topic$"));
    static CHAINE: LazyLock<Regex> = LazyLock::new(|| regex(r"(?i)(vevo|official)$"));

    let nom = chemin
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let titre_tag = tags.titre.clone().unwrap_or_default();
    let mut artistes = Vec::new();
    let mut titres = Vec::new();

    for brut in [&nom, &titre_tag] {
        if brut.is_empty() {
            continue;
        }
        let propre = nettoyer(brut);
        // « Artiste - Titre » : on coupe sur le premier tiret entouré d'espaces.
        let mut morceaux = TIRET.splitn(&propre, 2);
        let premier = morceaux.next().unwrap_or_default();
        let titre = match morceaux.next() {
            Some(titre) => {
                ajouter(&mut artistes, premier);
                sans_guillemets(titre).to_string()
            }
            None => sans_guillemets(&propre).to_string(),
        };
        // Un titre entre guillemets ailleurs dans le nom : 'Tequila Coma'.
        if let Some(entre) = GUILLEMETS.captures(brut) {
            ajouter(&mut titres, &entre[1]);
        }
        ajouter(&mut titres, &titre);
        ajouter(&mut titres, &sans_invites(&titre));
    }

    // L'artiste des tags (souvent « yeule - Topic » ou le nom de la chaîne YouTube).
    let artiste_tag = tags.artiste.clone().unwrap_or_default();
    let artiste_tag = TOPIC.replace(&artiste_tag, "");
    let artiste_tag = CHAINE.replace(&artiste_tag, "");
    ajouter(&mut artistes, artiste_tag.trim());
    // Rangement « Artiste/Album/morceau.mp3 » : le dossier au-dessus de l'album.
    let grand_parent = chemin
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    ajouter(&mut artistes, &grand_parent);

    let album = tags.album.clone().unwrap_or_else(|| {
        chemin
            .parent()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let duree = tags.duree.map_or(0.0, |d| d.as_secs_f64().round());
    Recherche {
        artistes,
        titres,
        album,
        duree,
    }
}

// ---------------------------------------------------------------- LRCLIB

/// Une réponse de LRCLIB qui a des paroles synchronisées.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidat {
    pub id: i64,
    pub artiste: String,
    pub titre: String,
    pub album: String,
    pub duree: f64,
    pub paroles: String,
}

/// Lit la réponse JSON d'une recherche LRCLIB. On ne garde que les
/// réponses qui ont des paroles synchronisées.
pub fn lire_reponse(json: &str) -> Vec<Candidat> {
    let Ok(serde_json::Value::Array(reponses)) = serde_json::from_str(json) else {
        return Vec::new();
    };
    reponses
        .iter()
        .filter_map(|r| {
            let texte = |cle: &str| {
                r.get(cle)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let paroles = texte("syncedLyrics");
            (!paroles.trim().is_empty()).then(|| Candidat {
                id: r.get("id").and_then(|v| v.as_i64()).unwrap_or_default(),
                artiste: texte("artistName"),
                titre: texte("trackName"),
                album: texte("albumName"),
                duree: r
                    .get("duration")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_default(),
                paroles,
            })
        })
        .collect()
}

/// La note d'une réponse, entre 0 et 1, et ses trois parties.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub totale: f64,
    pub titre: f64,
    pub artiste: f64,
    /// L'écart de durée, en secondes.
    pub ecart: f64,
}

/// Note une réponse : le titre compte beaucoup, l'artiste un peu, la durée
/// un peu aussi.
pub fn noter(candidat: &Candidat, recherche: &Recherche) -> Note {
    let titre_trouve = normaliser(&candidat.titre);
    let titre = recherche
        .titres
        .iter()
        .map(|t| ressemblance(&titre_trouve, &normaliser(t)))
        .fold(0.0, f64::max);
    let artiste_trouve = normaliser(&candidat.artiste);
    let artiste = if recherche.artistes.is_empty() {
        0.5
    } else {
        recherche
            .artistes
            .iter()
            .map(|a| {
                let a = normaliser(a);
                if !a.is_empty() && (artiste_trouve.contains(&a) || a.contains(&artiste_trouve)) {
                    1.0
                } else {
                    ressemblance(&artiste_trouve, &a)
                }
            })
            .fold(0.0, f64::max)
    };
    let ecart = (candidat.duree - recherche.duree).abs();
    let duree = if ecart <= 3.0 {
        1.0
    } else if ecart <= 10.0 {
        0.8
    } else if ecart <= 45.0 {
        0.5
    } else {
        0.1
    };
    Note {
        totale: 0.6 * titre + 0.25 * artiste + 0.15 * duree,
        titre,
        artiste,
        ecart,
    }
}

/// Les recherches à faire, dans l'ordre : titre + artiste, puis recherche
/// libre, et pour finir le titre seul.
pub fn recherches_a_faire(recherche: &Recherche) -> Vec<Vec<(&'static str, String)>> {
    let mut liste = Vec::new();
    for titre in &recherche.titres {
        for artiste in recherche.artistes.iter().take(2) {
            liste.push(vec![
                ("track_name", titre.clone()),
                ("artist_name", artiste.clone()),
            ]);
        }
        let libre = match recherche.artistes.first() {
            Some(artiste) => format!("{artiste} {titre}"),
            None => titre.clone(),
        };
        liste.push(vec![("q", libre)]);
    }
    for titre in recherche.titres.iter().take(2) {
        liste.push(vec![("track_name", titre.clone())]); // dernier recours
    }
    liste
}

/// Ce qu'a donné la recherche d'un morceau.
#[derive(Debug, PartialEq)]
pub enum Resultat {
    /// Trouvé. `a_verifier` : la durée est assez différente (version clip ?),
    /// la synchro est peut-être décalée.
    Trouve {
        paroles: String,
        source: String,
        a_verifier: bool,
    },
    /// Des réponses, mais aucune assez ressemblante : la meilleure.
    Rejete(String),
    /// Aucune réponse avec des paroles synchronisées.
    Introuvable,
}

/// Pourquoi une recherche s'est arrêtée avant la fin.
#[derive(Debug, PartialEq)]
pub enum Interruption {
    /// On a cliqué sur « Arrêter ».
    Arretee,
    /// LRCLIB n'a pas répondu.
    Reseau(String),
}

/// Cherche les paroles d'un morceau. `demander` interroge LRCLIB (ou, dans
/// les tests, fait semblant) ; `arreter` dit si on a cliqué sur « Arrêter ».
pub fn chercher(
    recherche: &Recherche,
    mut demander: impl FnMut(&[(&str, String)]) -> Result<Vec<Candidat>, String>,
    arreter: impl Fn() -> bool,
    pause: Duration,
) -> Result<Resultat, Interruption> {
    let mut trouves: Vec<Candidat> = Vec::new();
    for parametres in recherches_a_faire(recherche) {
        if arreter() {
            return Err(Interruption::Arretee);
        }
        for candidat in demander(&parametres).map_err(Interruption::Reseau)? {
            if !trouves.iter().any(|t| t.id == candidat.id) {
                trouves.push(candidat);
            }
        }
        // Arrêt anticipé si on a déjà une très bonne réponse.
        if trouves
            .iter()
            .any(|c| noter(c, recherche).totale >= NOTE_SUFFISANTE)
        {
            break;
        }
        std::thread::sleep(pause);
    }
    Ok(choisir(trouves, recherche))
}

/// Garde la meilleure réponse, si elle ressemble assez au morceau.
pub fn choisir(mut trouves: Vec<Candidat>, recherche: &Recherche) -> Resultat {
    // Tri stable : à égalité, la première réponse reçue reste devant.
    trouves.sort_by(|a, b| {
        noter(b, recherche)
            .totale
            .total_cmp(&noter(a, recherche).totale)
    });
    let Some(meilleur) = trouves.into_iter().next() else {
        return Resultat::Introuvable;
    };
    let note = noter(&meilleur, recherche);
    if note.titre < 0.75 || note.artiste < 0.6 {
        return Resultat::Rejete(format!("{} — {}", meilleur.artiste, meilleur.titre));
    }
    Resultat::Trouve {
        source: format!("LRCLIB : {} — {}", meilleur.artiste, meilleur.titre),
        paroles: meilleur.paroles,
        a_verifier: note.ecart > 10.0,
    }
}

/// Le texte du .lrc : un en-tête (artiste, titre, album, source), puis les
/// paroles.
pub fn contenu_du_lrc(chemin: &Path, recherche: &Recherche, paroles: &str, source: &str) -> String {
    let artiste = recherche.artistes.first().map_or("", String::as_str);
    let titre = match recherche.titres.first() {
        Some(titre) => titre.clone(),
        None => chemin
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    format!(
        "[ar:{artiste}]\n[ti:{titre}]\n[al:{}]\n[by:{source}]\n{}\n",
        recherche.album,
        paroles.trim()
    )
}

/// Interroge LRCLIB. Une page introuvable veut dire « aucune réponse ». En
/// cas d'erreur, on réessaie deux fois, après 1 puis 2 secondes.
fn demander_a_lrclib(
    agent: &ureq::Agent,
    parametres: &[(&str, String)],
) -> Result<Vec<Candidat>, String> {
    let mut essai = 0;
    loop {
        let mut requete = agent.get(format!("{API}/search"));
        for (cle, valeur) in parametres {
            requete = requete.query(*cle, valeur);
        }
        match requete.call() {
            Ok(mut reponse) => {
                let json = reponse
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| e.to_string())?;
                return Ok(lire_reponse(&json));
            }
            Err(ureq::Error::StatusCode(404)) => return Ok(Vec::new()),
            Err(erreur) if essai == 2 => return Err(erreur.to_string()),
            Err(_) => {}
        }
        essai += 1;
        std::thread::sleep(Duration::from_secs(essai));
    }
}

// ---------------------------------------------------------------- en arrière-plan

/// Où en est la génération des paroles.
#[derive(Debug, Default)]
pub struct Progression {
    /// Morceaux traités, sur le nombre total.
    pub faits: usize,
    pub total: usize,
    /// Le morceau qu'on cherche en ce moment.
    pub en_cours: Option<PathBuf>,
    /// Les .lrc créés (y compris ceux à vérifier).
    pub trouves: usize,
    /// Les morceaux qui avaient déjà un .lrc.
    pub deja_presents: usize,
    pub introuvables: Vec<PathBuf>,
    /// Trouvés, mais avec une durée assez différente (version clip ?).
    pub a_verifier: Vec<PathBuf>,
    /// LRCLIB n'a pas répondu, ou le .lrc n'a pas pu être écrit.
    pub erreurs: Vec<(PathBuf, String)>,
    /// Les .lrc écrits depuis la dernière fois qu'on a demandé : si c'est
    /// le morceau en cours, ses paroles s'affichent tout de suite.
    pub nouveaux: Vec<PathBuf>,
    /// Pourquoi on s'est arrêté avant la fin, s'il y a lieu.
    pub interruption: Option<String>,
    pub fini: bool,
}

/// La génération des paroles, qui tourne en arrière-plan.
pub struct Generation {
    progression: Arc<Mutex<Progression>>,
    arreter: Arc<AtomicBool>,
}

impl Generation {
    /// Cherche les paroles de ces morceaux, un par un, en arrière-plan.
    pub fn lancer(ctx: &egui::Context, morceaux: Vec<PathBuf>) -> Self {
        let progression = Arc::new(Mutex::new(Progression {
            total: morceaux.len(),
            ..Progression::default()
        }));
        let arreter = Arc::new(AtomicBool::new(false));
        let (pour_le_travail, stop, ctx) =
            (Arc::clone(&progression), Arc::clone(&arreter), ctx.clone());
        std::thread::spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .user_agent(AGENT)
                .timeout_global(Some(Duration::from_secs(20)))
                .build()
                .into();
            let Some(rangement) = Rangement::du_lecteur() else {
                let mut p = pour_le_travail.lock().unwrap_or_else(|e| e.into_inner());
                p.interruption = Some("Dossier des paroles introuvable.".to_string());
                p.fini = true;
                return;
            };
            generer(
                &morceaux,
                &rangement,
                &pour_le_travail,
                &stop,
                |p| demander_a_lrclib(&agent, p),
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

    /// Demande l'arrêt : le morceau en cours est abandonné.
    pub fn arreter(&self) {
        self.arreter.store(true, Ordering::Relaxed);
    }

    pub fn arret_demande(&self) -> bool {
        self.arreter.load(Ordering::Relaxed)
    }
}

/// Le travail lui-même : chaque morceau sans .lrc, l'un après l'autre.
fn generer(
    morceaux: &[PathBuf],
    rangement: &Rangement,
    progression: &Mutex<Progression>,
    arreter: &AtomicBool,
    mut demander: impl FnMut(&[(&str, String)]) -> Result<Vec<Candidat>, String>,
    pause: Duration,
    redessiner: impl Fn(),
) {
    let etat = || progression.lock().unwrap_or_else(|e| e.into_inner());
    let mut echecs_de_suite = 0;
    for chemin in morceaux {
        if arreter.load(Ordering::Relaxed) {
            etat().interruption = Some("Arrêté.".to_string());
            break;
        }
        if rangement.trouver(chemin).is_some() {
            let mut etat = etat();
            etat.deja_presents += 1;
            etat.faits += 1;
            continue;
        }
        etat().en_cours = Some(chemin.clone());
        redessiner();

        let recherche = preparer(chemin, &tags::lire(chemin));
        let resultat = chercher(
            &recherche,
            &mut demander,
            || arreter.load(Ordering::Relaxed),
            pause,
        );
        let mut etat = etat();
        match resultat {
            Ok(Resultat::Trouve {
                paroles,
                source,
                a_verifier,
            }) => {
                echecs_de_suite = 0;
                let texte = contenu_du_lrc(chemin, &recherche, &paroles, &source);
                match ecrire_sans_remplacer(&rangement.place(chemin), &texte) {
                    Ok(()) => {
                        etat.trouves += 1;
                        etat.nouveaux.push(chemin.clone());
                        if a_verifier {
                            etat.a_verifier.push(chemin.clone());
                        }
                    }
                    Err(erreur) => etat.erreurs.push((chemin.clone(), erreur.to_string())),
                }
            }
            Ok(Resultat::Rejete(_) | Resultat::Introuvable) => {
                echecs_de_suite = 0;
                etat.introuvables.push(chemin.clone());
            }
            Err(Interruption::Arretee) => {
                etat.interruption = Some("Arrêté.".to_string());
                break;
            }
            Err(Interruption::Reseau(erreur)) => {
                etat.erreurs.push((chemin.clone(), erreur));
                echecs_de_suite += 1;
                if echecs_de_suite >= ECHECS_DE_SUITE_MAX {
                    etat.faits += 1;
                    etat.interruption = Some(
                        "LRCLIB ne répond pas : la connexion Internet est peut-être coupée."
                            .to_string(),
                    );
                    break;
                }
            }
        }
        etat.faits += 1;
    }
    let mut etat = etat();
    etat.en_cours = None;
    etat.fini = true;
    drop(etat);
    redessiner();
}

/// Écrit le .lrc, sauf s'il existe déjà (créé entre-temps par un autre
/// logiciel) : on ne remplace jamais des paroles existantes. Crée les
/// sous-dossiers qui manquent.
fn ecrire_sans_remplacer(fichier: &Path, texte: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dossier) = fichier.parent() {
        std::fs::create_dir_all(dossier)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(fichier)?;
    f.write_all(texte.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paroles::fichier_lrc;

    /// Un rangement de test : la « Musique » et le dossier des paroles sont
    /// deux sous-dossiers de `dossier`.
    fn rangement_de_test(dossier: &Path) -> Rangement {
        Rangement {
            dossier: dossier.join("paroles"),
            musique: dossier.join("Musique"),
        }
    }

    fn tags(titre: &str, artiste: &str, album: &str, secondes: u64) -> Tags {
        let texte = |t: &str| (!t.is_empty()).then(|| t.to_string());
        let mut tags = Tags::default();
        tags.titre = texte(titre);
        tags.artiste = texte(artiste);
        tags.album = texte(album);
        tags.duree = Some(Duration::from_secs(secondes));
        tags
    }

    fn candidat(id: i64, artiste: &str, titre: &str, duree: f64) -> Candidat {
        Candidat {
            id,
            artiste: artiste.to_string(),
            titre: titre.to_string(),
            album: String::new(),
            duree,
            paroles: format!("[00:01.00]{titre}"),
        }
    }

    #[test]
    fn normalise_comme_le_script() {
        assert_eq!(normaliser("Déjà Vu (Remix)"), "deja vu remix");
        assert_eq!(normaliser("Simon & Garfunkel"), "simon and garfunkel");
        assert_eq!(normaliser("  Ça  va ?! "), "ca va");
        assert_eq!(normaliser("BLÅ_HÖR"), "bla_hor");
    }

    #[test]
    fn nettoie_les_titres_youtube() {
        assert_eq!(nettoyer("01. Titre"), "Titre");
        assert_eq!(nettoyer("07 - Titre"), "Titre");
        assert_eq!(nettoyer("3) Titre"), "Titre");
        assert_eq!(nettoyer("12 Titre"), "Titre");
        assert_eq!(
            nettoyer("Artiste - Titre (Official Video)"),
            "Artiste - Titre"
        );
        assert_eq!(nettoyer("Artiste - Titre [HD]"), "Artiste - Titre");
        assert_eq!(
            nettoyer("Artiste - Titre (Lyric Video) [4K]"),
            "Artiste - Titre"
        );
        assert_eq!(nettoyer("Titre | Chaîne YouTube"), "Titre");
        assert_eq!(nettoyer("Titre - Official Music Video"), "Titre");
        assert_eq!(nettoyer("Artiste_-_Titre_(Audio)"), "Artiste - Titre");
        // Ce qui n'est pas une mention reste.
        assert_eq!(
            nettoyer("Artiste - Titre (Remix)"),
            "Artiste - Titre (Remix)"
        );
        // Un nombre suivi de chiffres n'est pas un numéro de piste.
        assert_eq!(nettoyer("1999"), "1999");
    }

    #[test]
    fn retire_les_invites() {
        assert_eq!(sans_invites("Titre (feat. Quelqu'un)"), "Titre");
        assert_eq!(sans_invites("Titre ft. X"), "Titre");
        assert_eq!(sans_invites("Titre"), "Titre");
    }

    /// Les valeurs attendues viennent de Python :
    /// `difflib.SequenceMatcher(None, a, b).ratio()`.
    #[test]
    fn ressemblance_comme_python() {
        let cas = [
            ("abcd", "bcde", 0.75),
            ("tequila coma", "tequila", 0.736_842_105_263_157_9),
            (
                "bohemian rhapsody",
                "bohemian rapsody",
                0.969_696_969_696_969_7,
            ),
            ("abc", "xyz", 0.0),
            ("the beatles", "beatles the", 0.636_363_636_363_636_4),
            ("aaab", "abaa", 0.5),
            ("", "", 1.0),
        ];
        for (a, b, attendu) in cas {
            let r = ressemblance(a, b);
            assert!(
                (r - attendu).abs() < 1e-9,
                "{a} / {b} : {r} au lieu de {attendu}"
            );
        }
    }

    #[test]
    fn devine_artiste_et_titre_du_nom_de_fichier() {
        let chemin = Path::new("Musique/Inconnu/Clips/yeule - 'Tequila Coma' (Official Video).mp3");
        let r = preparer(chemin, &tags("", "yeule - Topic", "", 201));
        assert_eq!(r.artistes, ["yeule", "Inconnu"]);
        assert_eq!(r.titres, ["Tequila Coma"]);
        assert_eq!(r.album, "Clips");
        assert_eq!(r.duree, 201.0);
    }

    #[test]
    fn utilise_aussi_les_tags() {
        let chemin = Path::new("M/Daft Punk/Discovery/01 One More Time.mp3");
        let r = preparer(
            chemin,
            &tags(
                "One More Time (feat. Romanthony)",
                "Daft Punk",
                "Discovery",
                320,
            ),
        );
        assert_eq!(r.artistes, ["Daft Punk"]);
        assert_eq!(
            r.titres,
            ["One More Time", "One More Time (feat. Romanthony)"]
        );
        assert_eq!(r.album, "Discovery");
    }

    #[test]
    fn ordre_des_recherches() {
        let r = Recherche {
            artistes: vec!["A".into(), "B".into(), "C".into()],
            titres: vec!["T1".into(), "T2".into()],
            ..Recherche::default()
        };
        fn p(l: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
            l.iter().map(|(c, v)| (*c, v.to_string())).collect()
        }
        assert_eq!(
            recherches_a_faire(&r),
            vec![
                p(&[("track_name", "T1"), ("artist_name", "A")]),
                p(&[("track_name", "T1"), ("artist_name", "B")]),
                p(&[("q", "A T1")]),
                p(&[("track_name", "T2"), ("artist_name", "A")]),
                p(&[("track_name", "T2"), ("artist_name", "B")]),
                p(&[("q", "A T2")]),
                p(&[("track_name", "T1")]),
                p(&[("track_name", "T2")]),
            ]
        );
    }

    #[test]
    fn note_titre_artiste_duree() {
        let r = Recherche {
            artistes: vec!["Daft Punk".into()],
            titres: vec!["One More Time".into()],
            duree: 320.0,
            ..Recherche::default()
        };
        let parfait = noter(&candidat(1, "Daft Punk", "One More Time", 321.0), &r);
        assert!((parfait.totale - 1.0).abs() < 1e-9);
        let decale = noter(&candidat(2, "Daft Punk", "One More Time", 340.0), &r);
        assert!((decale.totale - (0.6 + 0.25 + 0.15 * 0.5)).abs() < 1e-9);
        // Artiste contenu dans un nom plus long : compte comme le même.
        let duo = noter(
            &candidat(3, "Daft Punk & Romanthony", "One More Time", 320.0),
            &r,
        );
        assert_eq!(duo.artiste, 1.0);
    }

    #[test]
    fn choisit_la_meilleure_ou_rejette() {
        let r = Recherche {
            artistes: vec!["Daft Punk".into()],
            titres: vec!["One More Time".into()],
            duree: 320.0,
            ..Recherche::default()
        };
        let trouve = choisir(
            vec![
                candidat(1, "Autre", "One More Time", 320.0),
                candidat(2, "Daft Punk", "One More Time", 340.0),
            ],
            &r,
        );
        assert_eq!(
            trouve,
            Resultat::Trouve {
                paroles: "[00:01.00]One More Time".into(),
                source: "LRCLIB : Daft Punk — One More Time".into(),
                a_verifier: true,
            }
        );
        assert_eq!(
            choisir(
                vec![candidat(1, "Daft Punk", "Around the World", 320.0)],
                &r
            ),
            Resultat::Rejete("Daft Punk — Around the World".into())
        );
        assert_eq!(choisir(Vec::new(), &r), Resultat::Introuvable);
    }

    #[test]
    fn lit_la_reponse_de_lrclib() {
        let json = r#"[
            {"id": 1, "trackName": "T", "artistName": "A", "albumName": "Al",
             "duration": 200.5, "syncedLyrics": "[00:01.00]la"},
            {"id": 2, "trackName": "T", "artistName": "A", "syncedLyrics": null},
            {"id": 3, "trackName": "T", "artistName": "A", "syncedLyrics": ""}
        ]"#;
        let lus = lire_reponse(json);
        assert_eq!(lus.len(), 1);
        assert_eq!(lus[0].duree, 200.5);
        assert_eq!(lus[0].album, "Al");
        assert!(lire_reponse("pas du json").is_empty());
    }

    #[test]
    fn s_arrete_des_qu_une_reponse_est_tres_bonne() {
        let r = Recherche {
            artistes: vec!["Daft Punk".into()],
            titres: vec!["One More Time".into(), "One More".into()],
            duree: 320.0,
            ..Recherche::default()
        };
        let mut appels = 0;
        let resultat = chercher(
            &r,
            |_| {
                appels += 1;
                Ok(vec![candidat(7, "Daft Punk", "One More Time", 320.0)])
            },
            || false,
            Duration::ZERO,
        );
        assert_eq!(appels, 1);
        assert!(matches!(
            resultat,
            Ok(Resultat::Trouve {
                a_verifier: false,
                ..
            })
        ));
        // Bouton « Arrêter » : on n'interroge plus LRCLIB.
        let resultat = chercher(
            &r,
            |_| panic!("ne doit pas chercher"),
            || true,
            Duration::ZERO,
        );
        assert_eq!(resultat, Err(Interruption::Arretee));
    }

    #[test]
    fn en_tete_du_lrc() {
        let r = Recherche {
            artistes: vec!["yeule".into()],
            titres: vec!["Tequila Coma".into()],
            album: "Clips".into(),
            duree: 201.0,
        };
        assert_eq!(
            contenu_du_lrc(Path::new("x.mp3"), &r, "\n[00:01.00]la\n\n", "LRCLIB : yeule — Tequila Coma"),
            "[ar:yeule]\n[ti:Tequila Coma]\n[al:Clips]\n[by:LRCLIB : yeule — Tequila Coma]\n[00:01.00]la\n"
        );
    }

    fn dossier_de_test(nom: &str) -> PathBuf {
        let dossier = std::env::temp_dir().join(format!("lecteur_mp3_generer_{nom}"));
        let _ = std::fs::remove_dir_all(&dossier);
        std::fs::create_dir_all(&dossier).unwrap();
        dossier
    }

    #[test]
    fn genere_sans_jamais_ecraser() {
        let dossier = dossier_de_test("ecraser");
        let rangement = rangement_de_test(&dossier);
        let album = rangement.musique.join("Daft Punk");
        std::fs::create_dir_all(&album).unwrap();
        let deja = album.join("Daft Punk - One More Time.mp3");
        let deja_range = album.join("Daft Punk - Aerodynamic.mp3");
        let nouveau = album.join("Daft Punk - Digital Love.mp3");
        let inconnu = album.join("Personne - Rien du tout.mp3");
        for f in [&deja, &deja_range, &nouveau, &inconnu] {
            std::fs::write(f, b"pas vraiment un mp3").unwrap();
        }
        std::fs::write(fichier_lrc(&deja), "mes paroles").unwrap();
        let place_deja_range = rangement.place(&deja_range);
        std::fs::create_dir_all(place_deja_range.parent().unwrap()).unwrap();
        std::fs::write(&place_deja_range, "déjà rangées").unwrap();

        let progression = Mutex::new(Progression::default());
        generer(
            &[
                deja.clone(),
                deja_range.clone(),
                nouveau.clone(),
                inconnu.clone(),
            ],
            &rangement,
            &progression,
            &AtomicBool::new(false),
            |p| {
                let titre = &p[0].1;
                Ok(if titre.contains("Digital Love") {
                    vec![candidat(1, "Daft Punk", "Digital Love", 0.0)]
                } else {
                    Vec::new()
                })
            },
            Duration::ZERO,
            || {},
        );
        let p = progression.into_inner().unwrap();
        assert_eq!(
            std::fs::read_to_string(fichier_lrc(&deja)).unwrap(),
            "mes paroles"
        );
        assert_eq!(
            std::fs::read_to_string(&place_deja_range).unwrap(),
            "déjà rangées"
        );
        // Le nouveau .lrc est dans notre dossier, pas dans la Musique.
        assert!(!fichier_lrc(&nouveau).exists());
        assert_eq!(
            rangement.place(&nouveau),
            dossier.join("paroles/Daft Punk/Daft Punk - Digital Love.lrc")
        );
        assert!(std::fs::read_to_string(rangement.place(&nouveau))
            .unwrap()
            .ends_with("[00:01.00]Digital Love\n"));
        assert!(!fichier_lrc(&inconnu).exists() && !rangement.place(&inconnu).exists());
        assert_eq!((p.faits, p.trouves, p.deja_presents), (4, 1, 2));
        assert_eq!(p.introuvables, [inconnu]);
        assert_eq!(p.nouveaux, [nouveau]);
        assert!(p.fini && p.interruption.is_none());
    }

    #[test]
    fn s_arrete_si_lrclib_ne_repond_pas() {
        let dossier = dossier_de_test("reseau");
        let morceaux: Vec<PathBuf> = (1..=5)
            .map(|n| dossier.join(format!("A - T{n}.mp3")))
            .collect();
        let progression = Mutex::new(Progression::default());
        let mut appels = 0;
        generer(
            &morceaux,
            &rangement_de_test(&dossier),
            &progression,
            &AtomicBool::new(false),
            |_| {
                appels += 1;
                Err("pas de connexion".to_string())
            },
            Duration::ZERO,
            || {},
        );
        let p = progression.into_inner().unwrap();
        assert_eq!(appels, ECHECS_DE_SUITE_MAX);
        assert_eq!(p.erreurs.len(), ECHECS_DE_SUITE_MAX);
        assert!(p.interruption.is_some() && p.fini);
    }
}
