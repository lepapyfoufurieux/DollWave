//! Un seul lecteur ouvert à la fois.
//!
//! Quand on double-clique sur un MP3, Windows (ou Linux) lance le lecteur
//! en lui donnant le chemin du fichier. Si un lecteur est déjà ouvert, ce
//! nouveau lancement lui transmet le fichier puis s'arrête aussitôt : on ne
//! se retrouve pas avec dix fenêtres en ouvrant dix MP3 d'un coup.
//!
//! Les deux lecteurs se parlent par une « prise locale » de la bibliothèque
//! `interprocess` (un tuyau nommé sous Windows, une prise Unix sous Linux) :
//! le premier lecteur l'ouvre et écoute, les suivants s'y branchent et
//! envoient leurs fichiers, un chemin par ligne.

use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use eframe::egui;
use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream};

/// Comment le lecteur démarre.
pub enum Demarrage {
    /// C'est le seul lecteur ouvert : il reçoit dans sa boîte les fichiers
    /// à ouvrir (ceux de son propre lancement, puis ceux des suivants).
    Premier(Boite),
    /// Un lecteur était déjà ouvert et a reçu les fichiers : on s'arrête.
    DejaOuvert,
}

/// Les fichiers que l'on demande au lecteur d'ouvrir, par lots (un lot par
/// lancement).
pub struct Boite {
    lots: Receiver<Vec<PathBuf>>,
    /// La fenêtre à réveiller quand un lot arrive (connue un peu plus tard).
    fenetre: Arc<OnceLock<egui::Context>>,
}

impl Boite {
    /// Une boîte qui ne recevra jamais rien.
    pub fn vide() -> Self {
        Self {
            lots: mpsc::channel().1,
            fenetre: Arc::default(),
        }
    }

    /// Désormais, l'arrivée d'un lot réveille cette fenêtre.
    pub fn brancher(&self, ctx: &egui::Context) {
        let _ = self.fenetre.set(ctx.clone());
        ctx.request_repaint();
    }

    /// Les lots arrivés depuis la dernière fois, dans l'ordre.
    pub fn recus(&self) -> Vec<Vec<PathBuf>> {
        self.lots.try_iter().collect()
    }
}

/// Démarre le lecteur avec les fichiers donnés au lancement (souvent un
/// seul, celui sur lequel on a double-cliqué ; aucun pour un lancement
/// normal).
pub fn demarrer(fichiers: Vec<PathBuf>) -> Demarrage {
    demarrer_sous(&nom_de_la_prise(), fichiers)
}

/// Le nom de la prise. Il contient le nom de l'utilisateur : sur un PC
/// partagé, chacun garde son propre lecteur.
fn nom_de_la_prise() -> String {
    let utilisateur = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let utilisateur: String = utilisateur
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    format!("lecteur_mp3-{utilisateur}.sock")
}

fn demarrer_sous(nom: &str, fichiers: Vec<PathBuf>) -> Demarrage {
    // Deux lancements presque en même temps (plusieurs MP3 ouverts d'un
    // coup) : un seul arrive à ouvrir la prise, les autres réessaient de
    // s'y brancher.
    for _ in 0..20 {
        if transmettre(nom, &fichiers).is_ok() {
            return Demarrage::DejaOuvert;
        }
        match ecouter(nom, fichiers.clone()) {
            Ok(boite) => return Demarrage::Premier(boite),
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                std::thread::sleep(Duration::from_millis(50));
            }
            // La prise ne marche pas du tout sur ce système : tant pis, le
            // lecteur s'ouvre normalement (sans recevoir les suivants).
            Err(_) => break,
        }
    }
    let (envoi, lots) = mpsc::channel();
    envoyer_s_il_y_en_a(&envoi, fichiers);
    Demarrage::Premier(Boite {
        lots,
        fenetre: Arc::default(),
    })
}

/// Envoie les fichiers au lecteur déjà ouvert, s'il y en a un.
fn transmettre(nom: &str, fichiers: &[PathBuf]) -> io::Result<()> {
    let mut prise = Stream::connect(nom.to_ns_name::<GenericNamespaced>()?)?;
    // Le lecteur déjà ouvert a le droit de passer au premier plan : sinon
    // Windows se contenterait de faire clignoter son bouton.
    super::autoriser_le_premier_plan();
    let mut texte = String::new();
    for fichier in fichiers {
        // Un chemin qui n'est pas de l'Unicode valide (très rare) est ignoré.
        if let Some(chemin) = fichier.to_str() {
            texte.push_str(chemin);
            texte.push('\n');
        }
    }
    // Une ligne vide marque la fin du lot.
    texte.push('\n');
    prise.write_all(texte.as_bytes())?;
    prise.flush()
}

/// Ouvre la prise et écoute, dans un fil à part, les lancements suivants.
fn ecouter(nom: &str, fichiers: Vec<PathBuf>) -> io::Result<Boite> {
    let prise = ListenerOptions::new()
        .name(nom.to_ns_name::<GenericNamespaced>()?)
        .create_sync()?;
    let (envoi, lots) = mpsc::channel();
    let fenetre: Arc<OnceLock<egui::Context>> = Arc::default();
    envoyer_s_il_y_en_a(&envoi, fichiers);
    let reveil = fenetre.clone();
    std::thread::Builder::new()
        .name("lancements suivants".into())
        .spawn(move || {
            for connexion in prise.incoming().filter_map(Result::ok) {
                if let Some(lot) = lire_un_lot(connexion) {
                    if envoi.send(lot).is_err() {
                        return; // le lecteur se ferme
                    }
                    if let Some(ctx) = reveil.get() {
                        ctx.request_repaint();
                    }
                }
            }
        })?;
    Ok(Boite { lots, fenetre })
}

/// Les fichiers du lancement du premier lecteur, s'il en a reçu.
fn envoyer_s_il_y_en_a(envoi: &mpsc::Sender<Vec<PathBuf>>, fichiers: Vec<PathBuf>) {
    if !fichiers.is_empty() {
        let _ = envoi.send(fichiers);
    }
}

/// Lit les chemins envoyés par un lancement, jusqu'à la ligne vide.
fn lire_un_lot(connexion: Stream) -> Option<Vec<PathBuf>> {
    let mut lot = Vec::new();
    for ligne in BufReader::new(connexion).lines() {
        let ligne = ligne.ok()?;
        if ligne.is_empty() {
            return Some(lot);
        }
        lot.push(PathBuf::from(ligne));
    }
    // Coupé avant la fin : on prend ce qui est arrivé.
    Some(lot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Attend qu'un lot arrive dans la boîte (au plus 5 secondes).
    fn attendre_un_lot(boite: &Boite) -> Vec<PathBuf> {
        let debut = Instant::now();
        loop {
            if let Some(lot) = boite.recus().into_iter().next() {
                return lot;
            }
            assert!(debut.elapsed() < Duration::from_secs(5), "rien reçu");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn le_deuxieme_lancement_transmet_ses_fichiers_au_premier() {
        let nom = format!("lecteur_mp3-test-{}.sock", std::process::id());
        let Demarrage::Premier(boite) = demarrer_sous(&nom, vec![PathBuf::from("a.mp3")]) else {
            panic!("le premier lancement doit ouvrir le lecteur");
        };
        // Ses propres fichiers arrivent en premier lot.
        assert_eq!(attendre_un_lot(&boite), [PathBuf::from("a.mp3")]);

        let deux = vec![PathBuf::from("Été 85/b.mp3"), PathBuf::from("c d.mp3")];
        assert!(matches!(
            demarrer_sous(&nom, deux.clone()),
            Demarrage::DejaOuvert
        ));
        assert_eq!(attendre_un_lot(&boite), deux);

        // Un lancement normal (sans fichier) envoie un lot vide.
        assert!(matches!(
            demarrer_sous(&nom, Vec::new()),
            Demarrage::DejaOuvert
        ));
        assert_eq!(attendre_un_lot(&boite), Vec::<PathBuf>::new());
    }

    #[test]
    fn une_boite_vide_ne_recoit_rien() {
        assert!(Boite::vide().recus().is_empty());
    }

    #[test]
    fn un_lancement_normal_ne_recoit_rien() {
        let nom = format!("lecteur_mp3-test-vide-{}.sock", std::process::id());
        let Demarrage::Premier(boite) = demarrer_sous(&nom, Vec::new()) else {
            panic!("le premier lancement doit ouvrir le lecteur");
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(boite.recus().is_empty());
    }
}
