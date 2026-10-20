//! La fenêtre « Modifier les tags » : corriger le titre, l'artiste, l'album
//! (…) d'un morceau et changer sa pochette. On l'ouvre d'un clic droit sur
//! un morceau, dans la liste de lecture ou dans la bibliothèque.

use std::path::{Path, PathBuf};

use eframe::egui;

use crate::ecriture_tags::{self, Champ, Modifications, Pochette};
use crate::liste::nom_du_fichier;
use crate::tags;

/// Côté de l'aperçu de la pochette, en points.
const APERCU: f32 = 120.0;

/// Ce que la fenêtre a donné à cette image.
#[derive(Debug, PartialEq)]
pub enum Issue {
    /// Toujours ouverte.
    Ouverte,
    /// Fermée sans rien changer.
    Annulee,
    /// Les tags ont été écrits dans le fichier : il faut le relire.
    Enregistree,
}

pub struct EditeurTags {
    chemin: PathBuf,
    /// Chaque champ : son nom à l'écran, sa valeur au départ, et celle tapée.
    champs: Vec<(Champ, &'static str, String, String)>,
    /// La pochette actuelle du fichier, à l'écran.
    pochette_actuelle: Option<egui::TextureHandle>,
    pochette: Pochette,
    /// La nouvelle pochette choisie, à l'écran.
    apercu_nouvelle: Option<egui::TextureHandle>,
    /// Le message d'erreur du dernier essai, s'il a échoué.
    erreur: Option<String>,
}

impl EditeurTags {
    /// Ouvre la fenêtre pour ce fichier, remplie avec ses tags actuels.
    pub fn ouvrir(ctx: &egui::Context, chemin: &Path) -> Self {
        let actuels = tags::lire(chemin);
        let texte = |valeur: &Option<String>| valeur.clone().unwrap_or_default();
        let champs = vec![
            (Champ::Titre, "Titre", texte(&actuels.titre)),
            (Champ::Artiste, "Artiste", texte(&actuels.artiste)),
            (Champ::Album, "Album", texte(&actuels.album)),
            (
                Champ::ArtisteAlbum,
                "Artiste de l'album",
                texte(&actuels.artiste_album),
            ),
            (
                Champ::Numero,
                "N° de piste",
                actuels.numero.map(|n| n.to_string()).unwrap_or_default(),
            ),
        ]
        .into_iter()
        .map(|(champ, nom, valeur)| (champ, nom, valeur.clone(), valeur))
        .collect();
        let pochette_actuelle = tags::lire_pochette(chemin)
            .map(|image| ctx.load_texture("pochette_editeur", image, egui::TextureOptions::LINEAR));
        Self {
            chemin: chemin.to_path_buf(),
            champs,
            pochette_actuelle,
            pochette: Pochette::Garder,
            apercu_nouvelle: None,
            erreur: None,
        }
    }

    pub fn chemin(&self) -> &Path {
        &self.chemin
    }

    /// Ce qui a changé par rapport au départ.
    fn modifications(&self) -> Modifications {
        Modifications {
            textes: self
                .champs
                .iter()
                .filter(|(_, _, depart, tape)| depart.trim() != tape.trim())
                .map(|(champ, _, _, tape)| (*champ, tape.clone()))
                .collect(),
            pochette: self.pochette.clone(),
        }
    }

    /// Dessine la fenêtre, au milieu du lecteur.
    pub fn afficher(&mut self, ctx: &egui::Context) -> Issue {
        let mut issue = Issue::Ouverte;
        let mut ouverte = true;
        egui::Window::new("✏ Modifier les tags")
            .open(&mut ouverte)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.weak(nom_du_fichier(&self.chemin));
                ui.add_space(6.0);
                ui.horizontal_top(|ui| {
                    egui::Grid::new("champs_des_tags")
                        .num_columns(2)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            for (_, nom, _, valeur) in &mut self.champs {
                                ui.label(*nom);
                                ui.add(egui::TextEdit::singleline(valeur).desired_width(260.0));
                                ui.end_row();
                            }
                        });
                    ui.add_space(12.0);
                    self.afficher_pochette(ui);
                });

                if let Some(erreur) = &self.erreur {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::LIGHT_RED, erreur);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let rien_a_faire = self.modifications() == Modifications::default();
                    let enregistrer = ui
                        .add_enabled(!rien_a_faire, egui::Button::new("💾 Enregistrer"))
                        .on_disabled_hover_text("Rien n'a été modifié");
                    if enregistrer.clicked() {
                        match ecriture_tags::enregistrer(&self.chemin, &self.modifications()) {
                            Ok(()) => issue = Issue::Enregistree,
                            Err(e) => self.erreur = Some(format!("Pas enregistré : {e}.")),
                        }
                    }
                    if ui.button("Annuler").clicked() {
                        issue = Issue::Annulee;
                    }
                });
            });
        if !ouverte || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            issue = Issue::Annulee;
        }
        issue
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
                    let (zone, _) = ui.allocate_exact_size(taille, egui::Sense::hover());
                    ui.painter().rect_stroke(
                        zone,
                        4.0,
                        ui.visuals().widgets.noninteractive.bg_stroke,
                    );
                    ui.painter().text(
                        zone.center(),
                        egui::Align2::CENTER_CENTER,
                        "Pas de pochette",
                        egui::FontId::proportional(12.0),
                        ui.visuals().weak_text_color(),
                    );
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .small_button("🖼 Changer…")
                    .on_hover_text("Choisir une image JPEG ou PNG")
                    .clicked()
                {
                    self.choisir_une_image(ui.ctx());
                }
                if a_montrer.is_some() && ui.small_button("Retirer").clicked() {
                    self.pochette = Pochette::Retirer;
                }
            });
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

    #[test]
    fn ne_renvoie_que_les_champs_changes() {
        let chemin = ecrire_mp3_avec(
            "editeur.mp3",
            &[("TIT2", "Titre"), ("TPE1", "Artiste"), ("TRCK", "4/10")],
            false,
            20,
        );
        let ctx = egui::Context::default();
        let mut editeur = EditeurTags::ouvrir(&ctx, &chemin);
        let valeurs: Vec<&str> = editeur.champs.iter().map(|c| c.3.as_str()).collect();
        assert_eq!(valeurs, ["Titre", "Artiste", "", "", "4"]);
        assert_eq!(editeur.modifications(), Modifications::default());

        editeur.champs[2].3 = "Nouvel album".into();
        // Ajouter des espaces au bout ne compte pas comme un changement.
        editeur.champs[0].3 = "Titre  ".into();
        assert_eq!(
            editeur.modifications().textes,
            [(Champ::Album, "Nouvel album".to_string())]
        );
    }
}
