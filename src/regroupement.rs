//! La bibliothèque rangée d'après les tags : par artiste (puis par album),
//! ou par album. Les morceaux sans tag vont dans « Artiste inconnu » et
//! « Album inconnu », tout en bas.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::liste::nom_du_fichier;
use crate::tags::Tags;

/// Un album, avec ses morceaux dans l'ordre du disque.
pub struct Album {
    /// Rien pour « Album inconnu ».
    pub nom: Option<String>,
    /// L'artiste affiché à côté du nom dans la vue Albums (« Artistes
    /// divers » pour une compilation ; rien s'il n'est pas connu).
    pub artiste: Option<String>,
    /// De quoi le reconnaître dans les favoris (voir `favori_album`).
    pub cle: PathBuf,
    pub morceaux: Vec<PathBuf>,
}

impl Album {
    pub fn nom(&self) -> &str {
        self.nom.as_deref().unwrap_or("Album inconnu")
    }
}

/// Un artiste, avec ses albums.
pub struct Artiste {
    /// Rien pour « Artiste inconnu ».
    pub nom: Option<String>,
    /// De quoi le reconnaître dans les favoris (voir `favori_artiste`).
    pub cle: PathBuf,
    pub albums: Vec<Album>,
    /// Le nombre de morceaux, tous albums confondus.
    pub total: usize,
}

impl Artiste {
    pub fn nom(&self) -> &str {
        self.nom.as_deref().unwrap_or("Artiste inconnu")
    }

    /// Tous ses morceaux, album par album, dans l'ordre affiché.
    pub fn morceaux(&self) -> Vec<PathBuf> {
        self.albums
            .iter()
            .flat_map(|a| a.morceaux.iter().cloned())
            .collect()
    }
}

/// La bibliothèque rangée des deux façons.
#[derive(Default)]
pub struct Regroupement {
    pub artistes: Vec<Artiste>,
    pub albums: Vec<Album>,
    /// Les morceaux dont les tags ne sont pas encore lus : ils ne sont
    /// rangés nulle part pour l'instant.
    pub pas_encore_lus: Vec<PathBuf>,
}

/// Des morceaux dont on a lu les tags, rassemblés sous un nom (rien pour
/// les inconnus).
type Groupe<'a> = (Option<String>, Vec<(&'a PathBuf, &'a Tags)>);

/// Les favoris sont des chemins de fichiers ; pour un artiste ou un album,
/// on fabrique un « chemin » qui ne peut pas exister sur le disque.
fn favori_artiste(cle: &str) -> PathBuf {
    PathBuf::from(format!("artiste:{cle}"))
}

fn favori_album(album: &str, artiste: &str) -> PathBuf {
    PathBuf::from(format!("album:{album}|{artiste}"))
}

/// Un nom pour comparer : sans majuscules ni espaces autour.
fn cle(nom: &str) -> String {
    nom.trim().to_lowercase()
}

/// Range les morceaux d'après leurs tags. `tags` donne ceux d'un morceau
/// s'ils sont déjà lus.
pub fn regrouper<'c, 't>(
    chemins: impl IntoIterator<Item = &'c PathBuf>,
    tags: impl Fn(&Path) -> Option<&'t Tags>,
) -> Regroupement {
    let mut pas_encore_lus = Vec::new();
    let mut lus: Vec<(&PathBuf, &Tags)> = Vec::new();
    for chemin in chemins {
        match tags(chemin) {
            Some(t) => lus.push((chemin, t)),
            None => pas_encore_lus.push(chemin.clone()),
        }
    }
    Regroupement {
        artistes: par_artiste(&lus),
        albums: par_album(&lus),
        pas_encore_lus,
    }
}

/// L'artiste sous lequel ranger un morceau : celui de l'album s'il est
/// indiqué, sinon celui du morceau.
fn artiste_du_morceau(tags: &Tags) -> Option<&str> {
    tags.artiste_album.as_deref().or(tags.artiste.as_deref())
}

/// La vue Artistes : chaque artiste, et dedans ses albums.
fn par_artiste(lus: &[(&PathBuf, &Tags)]) -> Vec<Artiste> {
    // Artiste (en minuscules, ou rien) → son nom tel qu'affiché et ses morceaux.
    let mut groupes: HashMap<Option<String>, Groupe> = HashMap::new();
    for &(chemin, tags) in lus {
        let nom = artiste_du_morceau(tags);
        groupes
            .entry(nom.map(cle))
            .or_insert_with(|| (nom.map(str::to_string), Vec::new()))
            .1
            .push((chemin, tags));
    }
    let mut artistes: Vec<Artiste> = groupes
        .into_iter()
        .map(|(cle_artiste, (nom, morceaux))| {
            let cle_artiste = cle_artiste.unwrap_or_default();
            let albums = rassembler_les_albums(&morceaux, |_, tags| {
                tags.album
                    .as_deref()
                    .map(|album| (cle(album), cle_artiste.clone()))
            });
            Artiste {
                total: morceaux.len(),
                cle: favori_artiste(&cle_artiste),
                nom,
                albums,
            }
        })
        .collect();
    trier(&mut artistes, |a| a.nom.as_deref());
    artistes
}

/// La vue Albums : tous les albums. Deux albums du même nom sont distincts
/// s'ils sont d'artistes différents (« Best Of »…). Pour une compilation
/// sans « artiste de l'album », on regarde plutôt le dossier : ses
/// morceaux, d'artistes différents, restent ensemble.
fn par_album(lus: &[(&PathBuf, &Tags)]) -> Vec<Album> {
    let mut resultat = rassembler_les_albums(lus, |chemin, tags| {
        let album = cle(tags.album.as_deref()?);
        let artiste = match &tags.artiste_album {
            Some(artiste) => cle(artiste),
            None => {
                let dossier = chemin.parent().unwrap_or(Path::new(""));
                format!("dossier:{}", cle(&dossier.to_string_lossy()))
            }
        };
        Some((album, artiste))
    });
    // L'artiste affiché : celui de l'album, sinon celui des morceaux s'ils
    // en ont tous le même, sinon « Artistes divers ».
    let tags_de: HashMap<&Path, &Tags> = lus.iter().map(|&(c, t)| (c.as_path(), t)).collect();
    for album in &mut resultat {
        if album.nom.is_none() {
            continue;
        }
        let artistes: Vec<Option<&str>> = album
            .morceaux
            .iter()
            .map(|m| tags_de.get(m.as_path()).and_then(|t| artiste_du_morceau(t)))
            .collect();
        let premier = artistes.iter().flatten().next().copied();
        album.artiste = premier.map(|premier| {
            if artistes
                .iter()
                .all(|a| a.is_some_and(|a| cle(a) == cle(premier)))
            {
                premier.to_string()
            } else {
                "Artistes divers".to_string()
            }
        });
    }
    trier(&mut resultat, |a| a.nom.as_deref());
    resultat
}

/// Rassemble des morceaux en albums. `album` donne, pour un morceau, de
/// quoi reconnaître son album (son nom et son artiste, ou son dossier, en
/// minuscules), ou
/// rien s'il n'en a pas : il va alors dans « Album inconnu », à la fin.
fn rassembler_les_albums(
    morceaux: &[(&PathBuf, &Tags)],
    album: impl Fn(&Path, &Tags) -> Option<(String, String)>,
) -> Vec<Album> {
    let mut groupes: HashMap<Option<(String, String)>, Groupe> = HashMap::new();
    for &(chemin, tags) in morceaux {
        groupes
            .entry(album(chemin, tags))
            .or_insert_with(|| (tags.album.clone(), Vec::new()))
            .1
            .push((chemin, tags));
    }
    let mut albums: Vec<Album> = groupes
        .into_iter()
        .map(|(cle_album, (nom, mut morceaux))| {
            match &cle_album {
                // Dans l'ordre du disque ; à numéro égal (ou sans numéro, à la
                // fin), par nom de fichier.
                Some(_) => morceaux.sort_by_cached_key(|(chemin, tags)| {
                    (
                        tags.disque.unwrap_or(1),
                        tags.numero.unwrap_or(u32::MAX),
                        nom_du_fichier(chemin).to_lowercase(),
                    )
                }),
                // Sans album, l'ordre du disque ne veut rien dire : par nom.
                None => morceaux
                    .sort_by_cached_key(|(chemin, tags)| tags.libelle(chemin).to_lowercase()),
            }
            let (cle, nom) = match cle_album {
                Some((album, artiste)) => (favori_album(&album, &artiste), nom),
                None => (PathBuf::from("album:"), None),
            };
            Album {
                nom,
                artiste: None,
                cle,
                morceaux: morceaux.into_iter().map(|(c, _)| c.clone()).collect(),
            }
        })
        .collect();
    trier(&mut albums, |a| a.nom.as_deref());
    albums
}

/// Par ordre alphabétique, sans tenir compte des majuscules ; les inconnus
/// (sans nom) à la fin.
fn trier<T>(elements: &mut [T], nom: impl Fn(&T) -> Option<&str>) {
    elements.sort_by_cached_key(|e| match nom(e) {
        Some(nom) => (false, cle(nom)),
        None => (true, String::new()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Des tags inventés (on ne lit aucun fichier ici).
    fn tags(
        artiste: Option<&str>,
        album: Option<&str>,
        artiste_album: Option<&str>,
        numero: Option<u32>,
    ) -> Tags {
        let mut tags = Tags::default();
        tags.artiste = artiste.map(str::to_string);
        tags.album = album.map(str::to_string);
        tags.artiste_album = artiste_album.map(str::to_string);
        tags.numero = numero;
        tags
    }

    /// Range des morceaux (chemin, tags) ; `None` = tags pas encore lus.
    fn ranger(morceaux: &[(&str, Option<Tags>)]) -> Regroupement {
        let chemins: Vec<PathBuf> = morceaux.iter().map(|(c, _)| PathBuf::from(c)).collect();
        let connus: HashMap<PathBuf, Tags> = morceaux
            .iter()
            .filter_map(|(c, t)| Some((PathBuf::from(c), t.clone()?)))
            .collect();
        regrouper(&chemins, |c| connus.get(c))
    }

    fn noms(chemins: &[PathBuf]) -> Vec<String> {
        chemins.iter().map(|c| nom_du_fichier(c)).collect()
    }

    #[test]
    fn par_artiste_puis_par_album_dans_l_ordre_du_disque() {
        let r = ranger(&[
            (
                "m/b.mp3",
                Some(tags(Some("Daft Punk"), Some("Discovery"), None, Some(2))),
            ),
            (
                "m/a.mp3",
                Some(tags(Some("daft punk "), Some("discovery"), None, Some(1))),
            ),
            (
                "m/c.mp3",
                Some(tags(Some("Daft Punk"), Some("Homework"), None, None)),
            ),
            (
                "m/d.mp3",
                Some(tags(Some("ABBA"), Some("Gold"), None, Some(3))),
            ),
            (
                "m/e.mp3",
                Some(tags(Some("Daft Punk"), Some("Discovery"), None, None)),
            ),
        ]);
        // Majuscules et espaces ignorés ; ordre alphabétique.
        let artistes: Vec<_> = r.artistes.iter().map(|a| a.nom()).collect();
        assert_eq!(artistes, ["ABBA", "Daft Punk"]);
        let daft = &r.artistes[1];
        assert_eq!(daft.total, 4);
        let albums: Vec<_> = daft.albums.iter().map(|a| a.nom()).collect();
        assert_eq!(albums, ["Discovery", "Homework"]);
        // Piste 1, piste 2, puis celle sans numéro.
        assert_eq!(noms(&daft.albums[0].morceaux), ["a.mp3", "b.mp3", "e.mp3"]);
        assert_eq!(noms(&daft.morceaux()), ["a.mp3", "b.mp3", "e.mp3", "c.mp3"]);
        assert!(r.pas_encore_lus.is_empty());
    }

    #[test]
    fn l_artiste_de_l_album_garde_les_invites_avec_l_album() {
        let r = ranger(&[
            (
                "m/1.mp3",
                Some(tags(
                    Some("Daft Punk"),
                    Some("Discovery"),
                    Some("Daft Punk"),
                    Some(1),
                )),
            ),
            (
                "m/2.mp3",
                Some(tags(
                    Some("Daft Punk feat. Romanthony"),
                    Some("Discovery"),
                    Some("Daft Punk"),
                    Some(2),
                )),
            ),
        ]);
        assert_eq!(r.artistes.len(), 1);
        assert_eq!(r.artistes[0].nom(), "Daft Punk");
        assert_eq!(r.albums.len(), 1);
        assert_eq!(r.albums[0].artiste.as_deref(), Some("Daft Punk"));
        assert_eq!(noms(&r.albums[0].morceaux), ["1.mp3", "2.mp3"]);
    }

    #[test]
    fn sans_tags_tout_en_bas() {
        let r = ranger(&[
            ("m/zz.mp3", Some(tags(Some("Zebda"), None, None, None))),
            (
                "m/x.mp3",
                Some(tags(None, Some("Inconnu au bataillon"), None, None)),
            ),
            ("m/vide.mp3", Some(Tags::default())),
            (
                "m/aa.mp3",
                Some(tags(Some("Zebda"), Some("Essence ordinaire"), None, None)),
            ),
        ]);
        let artistes: Vec<_> = r.artistes.iter().map(|a| a.nom()).collect();
        assert_eq!(artistes, ["Zebda", "Artiste inconnu"]);
        // Chez Zebda, le morceau sans album passe après ses albums.
        let albums: Vec<_> = r.artistes[0].albums.iter().map(|a| a.nom()).collect();
        assert_eq!(albums, ["Essence ordinaire", "Album inconnu"]);
        let inconnu = &r.artistes[1];
        assert_eq!(inconnu.total, 2);
        // Vue Albums : un seul « Album inconnu », à la fin.
        let albums: Vec<_> = r.albums.iter().map(|a| a.nom()).collect();
        assert_eq!(
            albums,
            ["Essence ordinaire", "Inconnu au bataillon", "Album inconnu"]
        );
        assert_eq!(noms(&r.albums[2].morceaux), ["vide.mp3", "zz.mp3"]);
        assert_eq!(r.albums[1].artiste, None);
    }

    #[test]
    fn deux_albums_du_meme_nom() {
        let r = ranger(&[
            (
                "a/1.mp3",
                Some(tags(Some("ABBA"), Some("Best Of"), None, Some(1))),
            ),
            (
                "b/1.mp3",
                Some(tags(Some("Queen"), Some("Best Of"), Some("Queen"), Some(1))),
            ),
            // Une compilation sans artiste de l'album : un seul album, même
            // si chaque morceau a son artiste.
            (
                "c/1.mp3",
                Some(tags(Some("X"), Some("Été 2026"), None, Some(1))),
            ),
            (
                "c/2.mp3",
                Some(tags(Some("Y"), Some("Été 2026"), None, Some(2))),
            ),
        ]);
        let albums: Vec<_> = r
            .albums
            .iter()
            .map(|a| (a.nom(), a.artiste.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(albums.len(), 3);
        assert!(albums.contains(&("Best Of", "ABBA")));
        assert!(albums.contains(&("Best Of", "Queen")));
        assert_eq!(albums[2], ("Été 2026", "Artistes divers"));
        assert_eq!(r.albums[2].morceaux.len(), 2);
        // Des favoris distincts.
        assert_ne!(r.albums[0].cle, r.albums[1].cle);
    }

    #[test]
    fn les_tags_pas_encore_lus_attendent() {
        let r = ranger(&[
            ("m/lu.mp3", Some(tags(Some("ABBA"), None, None, None))),
            ("m/pas lu.mp3", None),
        ]);
        assert_eq!(r.artistes.len(), 1);
        assert_eq!(r.artistes[0].total, 1);
        assert_eq!(noms(&r.pas_encore_lus), ["pas lu.mp3"]);
    }

    #[test]
    fn les_favoris_ne_se_melangent_pas() {
        let r = ranger(&[(
            "m/1.mp3",
            Some(tags(Some("Gold"), Some("Gold"), None, None)),
        )]);
        // Un artiste et un album du même nom n'ont pas la même clé.
        assert_ne!(r.artistes[0].cle, r.albums[0].cle);
        // Et la clé d'un artiste ne ressemble à aucun fichier.
        assert_eq!(r.artistes[0].cle, PathBuf::from("artiste:gold"));
    }
}
