//! La fenêtre « Modifier les tags » : corriger le titre, l'artiste, l'album
//! (…) d'un morceau et changer sa pochette, ou faire de même pour tous les
//! morceaux d'un dossier, d'un artiste ou d'un album. On l'ouvre d'un clic
//! droit, dans la liste de lecture ou dans la bibliothèque.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::ecriture_tags::{self, Champ, Modifications, Pochette};
use crate::liste::nom_du_fichier;
use crate::tags::{self, Tags, TagsConnus};

/// Côté de l'aperçu de la pochette, en points.
const APERCU: f32 = 120.0;

/// Ce que la fenêtre demande au lecteur après cette image.
#[derive(Debug, Default, PartialEq)]
pub struct Retour {
    /// La fenêtre est à fermer.
    pub fermer: bool,
    /// Ces fichiers ont été réécrits : il faut relire leurs tags.
    pub a_relire: Vec<PathBuf>,
}

/// Un champ de la fenêtre.
struct ChampEdite {
    champ: Champ,
    nom: &'static str,
    /// Sa valeur au départ ; rien si les morceaux en ont des différentes.
    depart: Option<String>,
    /// Ce qui est tapé dans la case.
    tape: String,
}

impl ChampEdite {
    fn new(champ: Champ, nom: &'static str, depart: Option<String>) -> Self {
        let tape = depart.clone().unwrap_or_default();
        Self {
            champ,
            nom,
            depart,
            tape,
        }
    }

    /// A-t-il été modifié ? Ajouter des espaces au bout ne compte pas. Quand
    /// les valeurs étaient différentes, la case laissée vide n'y touche pas.
    fn modifie(&self) -> bool {
        match &self.depart {
            Some(depart) => depart.trim() != self.tape.trim(),
            None => !self.tape.trim().is_empty(),
        }
    }
}

/// L'enregistrement en cours, en arrière-plan.
struct Ecriture {
    avancement: Arc<Mutex<Avancement>>,
    /// Le lecteur a-t-il déjà été prévenu des fichiers à relire ?
    signale: bool,
}

#[derive(Default)]
struct Avancement {
    faits: usize,
    /// Les fichiers qui n'ont pas pu être modifiés, et pourquoi.
    echecs: Vec<(PathBuf, String)>,
    fini: bool,
}

pub struct EditeurTags {
    chemins: Vec<PathBuf>,
    champs: Vec<ChampEdite>,
    /// La pochette actuelle du morceau, à l'écran (pour un seul morceau).
    pochette_actuelle: Option<egui::TextureHandle>,
    pochette: Pochette,
    /// La nouvelle pochette choisie, à l'écran.
    apercu_nouvelle: Option<egui::TextureHandle>,
    /// Le message d'erreur du dernier essai, s'il a échoué.
    erreur: Option<String>,
    ecriture: Option<Ecriture>,
}

/// La valeur que tous ces morceaux ont en commun (vide si aucun n'en a),
/// ou rien s'ils en ont des différentes.
fn valeur_commune(valeurs: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let mut valeurs = valeurs.into_iter().map(Option::unwrap_or_default);
    let premiere = valeurs.next().unwrap_or_default();
    valeurs.all(|v| v == premiere).then_some(premiere)
}

impl EditeurTags {
    /// Ouvre la fenêtre pour ces fichiers, remplie avec leurs tags actuels.
    /// Pour plusieurs morceaux, seuls l'artiste, l'album, l'artiste de
    /// l'album et la pochette sont proposés (le titre et le numéro sont
    /// propres à chaque morceau).
    pub fn ouvrir(ctx: &egui::Context, chemins: Vec<PathBuf>, connus: &TagsConnus) -> Self {
        let (champs, pochette_actuelle) = if let [chemin] = chemins.as_slice() {
            // Un seul morceau : on relit le fichier, pour être sûr d'avoir ses
            // tags du moment.
            let actuels = tags::lire(chemin);
            let champs = vec![
                ChampEdite::new(Champ::Titre, "Titre", Some(texte(&actuels.titre))),
                ChampEdite::new(Champ::Artiste, "Artiste", Some(texte(&actuels.artiste))),
                ChampEdite::new(Champ::Album, "Album", Some(texte(&actuels.album))),
                ChampEdite::new(
                    Champ::ArtisteAlbum,
                    "Artiste de l'album",
                    Some(texte(&actuels.artiste_album)),
                ),
                ChampEdite::new(
                    Champ::Numero,
                    "N° de piste",
                    Some(actuels.numero.map(|n| n.to_string()).unwrap_or_default()),
                ),
            ];
            let pochette = tags::lire_pochette(chemin).map(|image| {
                ctx.load_texture("pochette_editeur", image, egui::TextureOptions::LINEAR)
            });
            (champs, pochette)
        } else {
            // Plusieurs morceaux : les tags déjà lus en arrière-plan (toute la
            // bibliothèque l'est), sinon on lit le fichier.
            let tous: Vec<Tags> = {
                let textes = connus.pour_chercher();
                chemins
                    .iter()
                    .map(|c| textes.tags(c).cloned().unwrap_or_else(|| tags::lire(c)))
                    .collect()
            };
            let commune = |valeur: fn(&Tags) -> &Option<String>| {
                valeur_commune(tous.iter().map(|t| valeur(t).clone()))
            };
            let champs = vec![
                ChampEdite::new(Champ::Artiste, "Artiste", commune(|t| &t.artiste)),
                ChampEdite::new(Champ::Album, "Album", commune(|t| &t.album)),
                ChampEdite::new(
                    Champ::ArtisteAlbum,
                    "Artiste de l'album",
                    commune(|t| &t.artiste_album),
                ),
            ];
            (champs, None)
        };
        Self {
            chemins,
            champs,
            pochette_actuelle,
            pochette: Pochette::Garder,
            apercu_nouvelle: None,
            erreur: None,
            ecriture: None,
        }
    }

    fn plusieurs(&self) -> bool {
        self.chemins.len() > 1
    }

    /// Ce qui a changé par rapport au départ.
    fn modifications(&self) -> Modifications {
        Modifications {
            textes: self
                .champs
                .iter()
                .filter(|c| c.modifie())
                .map(|c| (c.champ, c.tape.clone()))
                .collect(),
            pochette: self.pochette.clone(),
        }
    }

    /// Lance l'écriture de tous les fichiers, en arrière-plan : la fenêtre
    /// ne se fige pas, même pour un gros dossier.
    fn enregistrer(&mut self, ctx: &egui::Context) {
        let avancement = Arc::new(Mutex::new(Avancement::default()));
        let pour_le_fil = Arc::clone(&avancement);
        let chemins = self.chemins.clone();
        let modifications = self.modifications();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            ecrire_tous(&chemins, &modifications, &pour_le_fil);
            ctx.request_repaint();
        });
        self.erreur = None;
        self.ecriture = Some(Ecriture {
            avancement,
            signale: false,
        });
    }

    /// Dessine la fenêtre, au milieu du lecteur.
    pub fn afficher(&mut self, ctx: &egui::Context) -> Retour {
        let mut retour = Retour::default();
        let en_cours = self.suivre_l_ecriture(&mut retour);
        let mut ouverte = true;
        let titre = if self.plusieurs() {
            format!("✏ Modifier les tags de {} morceaux", self.chemins.len())
        } else {
            "✏ Modifier les tags".to_string()
        };
        egui::Window::new(titre)
            .id(egui::Id::new("editeur_tags"))
            .open(&mut ouverte)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                if self.ecriture.is_some() {
                    if self.afficher_avancement(ui) {
                        retour.fermer = true;
                    }
                } else if self.afficher_formulaire(ui) {
                    retour.fermer = true;
                }
            });
        // Pendant l'écriture, on ne ferme pas : on attend la fin.
        if !en_cours && (!ouverte || ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
            retour.fermer = true;
        }
        retour
    }

    /// Regarde où en est l'écriture. Une fois finie, le lecteur relit les
    /// fichiers ; pour un seul morceau réussi, la fenêtre se ferme, et en
    /// cas d'échec on revient au formulaire avec le message. Renvoie vrai
    /// tant que l'écriture n'est pas finie.
    fn suivre_l_ecriture(&mut self, retour: &mut Retour) -> bool {
        let plusieurs = self.plusieurs();
        let Some(ecriture) = &mut self.ecriture else {
            return false;
        };
        let premier_echec = {
            let avancement = ecriture
                .avancement
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !avancement.fini {
                return true;
            }
            avancement.echecs.first().map(|(_, erreur)| erreur.clone())
        };
        if !ecriture.signale {
            ecriture.signale = true;
            retour.a_relire = self.chemins.clone();
        }
        if !plusieurs {
            match premier_echec {
                Some(erreur) => self.erreur = Some(format!("Pas enregistré : {erreur}.")),
                None => retour.fermer = true,
            }
            self.ecriture = None;
        }
        false
    }

    /// Les champs, la pochette et les boutons. Renvoie vrai si on a annulé.
    fn afficher_formulaire(&mut self, ui: &mut egui::Ui) -> bool {
        if self.plusieurs() {
            ui.weak("Laisse une case telle quelle pour que chaque morceau garde la sienne.");
        } else {
            ui.weak(nom_du_fichier(&self.chemins[0]));
        }
        ui.add_space(6.0);
        ui.horizontal_top(|ui| {
            egui::Grid::new("champs_des_tags")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    for champ in &mut self.champs {
                        ui.label(champ.nom);
                        let mut case =
                            egui::TextEdit::singleline(&mut champ.tape).desired_width(260.0);
                        if champ.depart.is_none() {
                            case = case.hint_text("(plusieurs valeurs)");
                        }
                        ui.add(case);
                        ui.end_row();
                    }
                });
            ui.add_space(12.0);
            self.afficher_pochette(ui);
        });

        if let Some(erreur) = &self.erreur {
            ui.add_space(6.0);
            ui.colored_label(crate::theme::ERREUR, erreur);
        }

        ui.add_space(8.0);
        let mut annule = false;
        ui.horizontal(|ui| {
            let rien_a_faire = self.modifications() == Modifications::default();
            let enregistrer = ui
                .add_enabled(!rien_a_faire, egui::Button::new("💾 Enregistrer"))
                .on_disabled_hover_text("Rien n'a été modifié");
            if enregistrer.clicked() {
                self.enregistrer(ui.ctx());
            }
            if ui.button("Annuler").clicked() {
                annule = true;
            }
        });
        annule
    }

    /// L'avancement de l'écriture (« 12 / 40 »), puis le résumé. Renvoie
    /// vrai si on a cliqué sur « Fermer ».
    fn afficher_avancement(&self, ui: &mut egui::Ui) -> bool {
        let Some(ecriture) = &self.ecriture else {
            return false;
        };
        let avancement = ecriture
            .avancement
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let total = self.chemins.len();
        if !avancement.fini {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Enregistrement… {} / {total}", avancement.faits));
            });
            ui.add(
                egui::ProgressBar::new(avancement.faits as f32 / total.max(1) as f32)
                    .desired_width(380.0),
            );
            return false;
        }
        let reussis = total - avancement.echecs.len();
        if avancement.echecs.is_empty() {
            ui.label(format!("✔ {reussis} morceaux modifiés."));
        } else {
            ui.label(format!(
                "{reussis} modifiés, {} non modifiés :",
                avancement.echecs.len()
            ));
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    for (chemin, erreur) in &avancement.echecs {
                        ui.colored_label(
                            crate::theme::ERREUR,
                            format!("{} : {erreur}", nom_du_fichier(chemin)),
                        );
                    }
                });
        }
        ui.add_space(8.0);
        ui.button("Fermer").clicked()
    }

    /// L'aperçu de la pochette (la nouvelle si on en a choisi une), avec
    /// « Changer… » et « Retirer ».
    fn afficher_pochette(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            let a_montrer = match self.pochette {
                Pochette::Garder => self.pochette_actuelle.as_ref(),
                Pochette::Remplacer(_) => self.apercu_nouvelle.as_ref(),
                Pochette::Retirer => None,
            }
            .cloned();
            let taille = egui::vec2(APERCU, APERCU);
            match &a_montrer {
                Some(texture) => {
                    ui.add(egui::Image::new(texture).fit_to_exact_size(taille));
                }
                None => {
                    let texte = match self.pochette {
                        Pochette::Garder if self.plusieurs() => "Chacun garde\nla sienne",
                        _ => "Pas de pochette",
                    };
                    let (zone, _) = ui.allocate_exact_size(taille, egui::Sense::hover());
                    ui.painter().rect_stroke(
                        zone,
                        4.0,
                        ui.visuals().widgets.noninteractive.bg_stroke,
                    );
                    ui.painter().text(
                        zone.center(),
                        egui::Align2::CENTER_CENTER,
                        texte,
                        egui::FontId::proportional(12.0),
                        ui.visuals().weak_text_color(),
                    );
                }
            }
            // Les boutons l'un sous l'autre, de la largeur de l'aperçu : côte
            // à côte, ils dépassaient du bord de la fenêtre.
            let bouton = |texte| {
                egui::Button::new(texte)
                    .small()
                    .min_size(egui::vec2(APERCU, 0.0))
            };
            if ui
                .add(bouton("🖼 Changer…"))
                .on_hover_text("Choisir une image JPEG ou PNG")
                .clicked()
            {
                self.choisir_une_image(ui.ctx());
            }
            // Pour plusieurs morceaux, on peut retirer leurs pochettes
            // même sans les voir.
            let peut_retirer =
                a_montrer.is_some() || (self.plusieurs() && self.pochette == Pochette::Garder);
            if peut_retirer
                && ui
                    .add(bouton("Retirer"))
                    .on_hover_text("Plus de pochette dans le fichier")
                    .clicked()
            {
                self.pochette = Pochette::Retirer;
            }
        });
    }

    fn choisir_une_image(&mut self, ctx: &egui::Context) {
        let Some(fichier) = rfd::FileDialog::new()
            .add_filter("Images", &["jpg", "jpeg", "png"])
            .pick_file()
        else {
            return; // l'utilisateur a annulé
        };
        let lue = std::fs::read(&fichier)
            .ok()
            .and_then(|donnees| Some((apercu(&donnees)?, donnees)));
        match lue {
            Some((image, donnees)) => {
                self.apercu_nouvelle =
                    Some(ctx.load_texture("pochette_choisie", image, egui::TextureOptions::LINEAR));
                self.pochette = Pochette::Remplacer(donnees);
                self.erreur = None;
            }
            None => {
                self.erreur = Some(format!(
                    "« {} » n'est pas une image JPEG ou PNG lisible.",
                    nom_du_fichier(&fichier)
                ));
            }
        }
    }
}

fn texte(valeur: &Option<String>) -> String {
    valeur.clone().unwrap_or_default()
}

/// Écrit les modifications dans chaque fichier, l'un après l'autre. Un
/// fichier qui échoue n'arrête pas les autres.
fn ecrire_tous(chemins: &[PathBuf], modifications: &Modifications, avancement: &Mutex<Avancement>) {
    for chemin in chemins {
        let resultat = ecriture_tags::enregistrer(chemin, modifications);
        let mut avancement = avancement.lock().unwrap_or_else(|e| e.into_inner());
        avancement.faits += 1;
        if let Err(e) = resultat {
            avancement.echecs.push((chemin.clone(), e.to_string()));
        }
    }
    avancement.lock().unwrap_or_else(|e| e.into_inner()).fini = true;
}

/// L'image réduite pour l'aperçu, ou rien si ce n'est pas une image lisible.
fn apercu(donnees: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory(donnees).ok()?;
    let image = image.thumbnail(256, 256).to_rgba8();
    let taille = [image.width() as usize, image.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        taille,
        image.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::tests::ecrire_mp3_avec;
    use std::time::{Duration, Instant};

    fn valeurs(editeur: &EditeurTags) -> Vec<&str> {
        editeur.champs.iter().map(|c| c.tape.as_str()).collect()
    }

    /// Attend la fin de l'écriture, comme le ferait la fenêtre image après image.
    fn attendre_l_ecriture(editeur: &mut EditeurTags) -> Retour {
        let debut = Instant::now();
        loop {
            let mut retour = Retour::default();
            if !editeur.suivre_l_ecriture(&mut retour) {
                return retour;
            }
            assert!(
                debut.elapsed() < Duration::from_secs(5),
                "écriture sans fin"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn ne_renvoie_que_les_champs_changes() {
        let chemin = ecrire_mp3_avec(
            "editeur.mp3",
            &[("TIT2", "Titre"), ("TPE1", "Artiste"), ("TRCK", "4/10")],
            false,
            20,
        );
        let ctx = egui::Context::default();
        let connus = TagsConnus::new(&ctx);
        let mut editeur = EditeurTags::ouvrir(&ctx, vec![chemin], &connus);
        assert_eq!(valeurs(&editeur), ["Titre", "Artiste", "", "", "4"]);
        assert_eq!(editeur.modifications(), Modifications::default());

        editeur.champs[2].tape = "Nouvel album".into();
        // Ajouter des espaces au bout ne compte pas comme un changement.
        editeur.champs[0].tape = "Titre  ".into();
        assert_eq!(
            editeur.modifications().textes,
            [(Champ::Album, "Nouvel album".to_string())]
        );
    }

    #[test]
    fn un_morceau_enregistre_se_ferme_et_se_relit() {
        let chemin = ecrire_mp3_avec("editeur seul.mp3", &[("TIT2", "Avant")], false, 20);
        let ctx = egui::Context::default();
        let connus = TagsConnus::new(&ctx);
        let mut editeur = EditeurTags::ouvrir(&ctx, vec![chemin.clone()], &connus);
        editeur.champs[0].tape = "Après".into();
        editeur.enregistrer(&ctx);
        let retour = attendre_l_ecriture(&mut editeur);
        assert_eq!(
            retour,
            Retour {
                fermer: true,
                a_relire: vec![chemin.clone()]
            }
        );
        assert_eq!(tags::lire(&chemin).titre.as_deref(), Some("Après"));
    }

    #[test]
    fn plusieurs_morceaux_valeurs_communes_ou_differentes() {
        let a = ecrire_mp3_avec(
            "groupe a.mp3",
            &[("TIT2", "Un"), ("TPE1", "Daft Punk"), ("TALB", "Discovery")],
            false,
            20,
        );
        let b = ecrire_mp3_avec(
            "groupe b.mp3",
            &[
                ("TIT2", "Deux"),
                ("TPE1", "Daft Punk"),
                ("TALB", "Homework"),
            ],
            false,
            20,
        );
        let ctx = egui::Context::default();
        let connus = TagsConnus::new(&ctx);
        let mut editeur = EditeurTags::ouvrir(&ctx, vec![a.clone(), b.clone()], &connus);
        // Ni titre ni numéro : ils sont propres à chaque morceau.
        let champs: Vec<Champ> = editeur.champs.iter().map(|c| c.champ).collect();
        assert_eq!(champs, [Champ::Artiste, Champ::Album, Champ::ArtisteAlbum]);
        // L'artiste est le même pour les deux ; les albums diffèrent.
        assert_eq!(valeurs(&editeur), ["Daft Punk", "", ""]);
        assert_eq!(editeur.champs[1].depart, None);
        assert_eq!(editeur.champs[2].depart, Some(String::new()));
        assert_eq!(editeur.modifications(), Modifications::default());

        // On ne change que l'artiste de l'album : les albums restent les leurs.
        editeur.champs[2].tape = "Daft Punk".into();
        editeur.enregistrer(&ctx);
        let retour = attendre_l_ecriture(&mut editeur);
        // Le résumé reste affiché jusqu'à « Fermer ».
        assert!(!retour.fermer);
        assert_eq!(retour.a_relire, [a.clone(), b.clone()]);
        let (ta, tb) = (tags::lire(&a), tags::lire(&b));
        assert_eq!(ta.artiste_album.as_deref(), Some("Daft Punk"));
        assert_eq!(tb.artiste_album.as_deref(), Some("Daft Punk"));
        assert_eq!(ta.album.as_deref(), Some("Discovery"));
        assert_eq!(tb.album.as_deref(), Some("Homework"));
        assert_eq!(ta.titre.as_deref(), Some("Un"));
        // Les fichiers ne sont signalés qu'une fois.
        assert_eq!(attendre_l_ecriture(&mut editeur), Retour::default());
    }

    #[test]
    fn un_fichier_qui_echoue_n_arrete_pas_les_autres() {
        let bon = ecrire_mp3_avec("groupe bon.mp3", &[("TALB", "Vieux")], false, 20);
        let absent = std::env::temp_dir().join("dollwave_tests_tags/groupe absent.mp3");
        let bon2 = ecrire_mp3_avec("groupe bon 2.mp3", &[("TALB", "Vieux")], false, 20);
        let ctx = egui::Context::default();
        let connus = TagsConnus::new(&ctx);
        let mut editeur = EditeurTags::ouvrir(
            &ctx,
            vec![bon.clone(), absent.clone(), bon2.clone()],
            &connus,
        );
        editeur.champs[1].tape = "Neuf".into();
        editeur.enregistrer(&ctx);
        attendre_l_ecriture(&mut editeur);
        assert_eq!(tags::lire(&bon).album.as_deref(), Some("Neuf"));
        assert_eq!(tags::lire(&bon2).album.as_deref(), Some("Neuf"));
        let avancement = editeur
            .ecriture
            .as_ref()
            .unwrap()
            .avancement
            .lock()
            .unwrap();
        assert_eq!(avancement.faits, 3);
        assert_eq!(avancement.echecs.len(), 1);
        assert_eq!(avancement.echecs[0].0, absent);
    }

    #[test]
    fn valeur_commune_de_plusieurs_morceaux() {
        let v = |s: &str| Some(s.to_string());
        assert_eq!(valeur_commune([v("A"), v("A")]), v("A"));
        assert_eq!(valeur_commune([v("A"), v("B")]), None);
        assert_eq!(valeur_commune([None, None]), v(""));
        assert_eq!(valeur_commune([v("A"), None]), None);
    }
}
