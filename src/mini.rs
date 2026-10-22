//! Le mini-lecteur : la fenêtre du lecteur se transforme en une petite
//! fenêtre sans barre de titre, toujours au-dessus des autres, avec la
//! pochette, le titre, l'artiste, une barre de progression et les boutons
//! ⏮ / Lecture-Pause / ⏭. « Agrandir » la remet comme avant.
//!
//! Tout passe par les commandes de fenêtre d'egui : rien de propre à Windows,
//! le mini-lecteur marche pareil sous Linux.

use eframe::egui::{self, Pos2, Rect, Vec2, ViewportCommand};

use crate::bibliotheque::memoriser;
use crate::systeme::fichier_de_reglage;
use crate::theme::{self, Allure, Bouton, Coins, Icone};

/// La taille du mini-lecteur (sans bord : il n'a pas de barre de titre).
pub const TAILLE: Vec2 = Vec2::new(420.0, 128.0);
/// La taille minimale de la grande fenêtre (les deux colonnes du bas
/// seraient trop serrées en dessous).
pub const TAILLE_MIN_GRANDE: Vec2 = Vec2::new(1040.0, 720.0);
/// Le fichier où l'on mémorise la place du mini-lecteur à l'écran.
const FICHIER: &str = "mini_lecteur.txt";
/// Côté de la pochette.
const COTE_POCHETTE: f32 = 108.0;

/// Ce que l'on demande depuis le mini-lecteur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Precedent,
    LecturePause,
    Suivant,
    /// Aller à cet endroit du morceau (de 0.0 au début à 1.0 à la fin).
    Sauter(f32),
    Agrandir,
}

/// Ce que le mini-lecteur affiche.
pub struct Affichage<'a> {
    pub pochette: Option<&'a egui::TextureHandle>,
    pub titre: String,
    pub artiste: Option<String>,
    /// Où on en est dans le morceau, de 0.0 à 1.0.
    pub fraction: f32,
    /// « 1:23 / 3:45 ».
    pub temps: String,
    /// Le morceau tourne : le bouton du milieu montre Pause.
    pub en_lecture: bool,
    /// La liste contient des morceaux : les boutons sont actifs.
    pub boutons_actifs: bool,
    /// Un morceau est ouvert : on peut cliquer sur la barre pour sauter.
    pub peut_sauter: bool,
}

/// L'état du mini-lecteur : affiché ou non, et les places de la fenêtre.
#[derive(Default)]
pub struct MiniLecteur {
    actif: bool,
    /// La grande fenêtre avant de passer en mini : le coin en haut à gauche
    /// de la fenêtre (barre de titre comprise) et la taille de son intérieur.
    grande: Option<Rect>,
    /// La place du mini-lecteur à l'écran (coin en haut à gauche).
    position: Option<Pos2>,
}

impl MiniLecteur {
    /// Le mini-lecteur, avec la place mémorisée la dernière fois.
    pub fn charger() -> Self {
        let texte = fichier_de_reglage(FICHIER).and_then(|f| std::fs::read_to_string(f).ok());
        MiniLecteur {
            position: texte.as_deref().and_then(lire_position),
            ..Default::default()
        }
    }

    pub fn actif(&self) -> bool {
        self.actif
    }

    /// Passe en mini-lecteur, en notant la place de la grande fenêtre pour
    /// la retrouver en agrandissant.
    pub fn entrer(&mut self, ctx: &egui::Context) {
        if self.actif {
            return;
        }
        let (dehors, dedans) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        self.grande = dehors
            .zip(dedans)
            .map(|(dehors, dedans)| Rect::from_min_size(dehors.min, dedans.size()));
        self.actif = true;
        for commande in self.commandes_pour_entrer() {
            ctx.send_viewport_cmd(commande);
        }
    }

    /// Revient à la grande fenêtre, et mémorise la place du mini-lecteur.
    pub fn sortir(&mut self, ctx: &egui::Context) {
        if !self.actif {
            return;
        }
        self.suivre_la_position(ctx);
        self.memoriser();
        self.actif = false;
        for commande in self.commandes_pour_sortir() {
            ctx.send_viewport_cmd(commande);
        }
    }

    /// Les commandes de fenêtre pour passer en mini : on baisse d'abord la
    /// taille minimale, sinon la fenêtre refuserait de rapetisser.
    fn commandes_pour_entrer(&self) -> Vec<ViewportCommand> {
        let mut commandes = vec![
            ViewportCommand::Maximized(false),
            ViewportCommand::Decorations(false),
            ViewportCommand::Resizable(false),
            ViewportCommand::MinInnerSize(TAILLE),
            ViewportCommand::InnerSize(TAILLE),
            ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop),
        ];
        if let Some(position) = self.position {
            commandes.push(ViewportCommand::OuterPosition(position));
        }
        commandes
    }

    /// Les commandes de fenêtre pour revenir en grand, à la place d'avant.
    fn commandes_pour_sortir(&self) -> Vec<ViewportCommand> {
        let mut commandes = vec![
            ViewportCommand::WindowLevel(egui::WindowLevel::Normal),
            ViewportCommand::Decorations(true),
            ViewportCommand::Resizable(true),
        ];
        let taille = self
            .grande
            .map_or(TAILLE_MIN_GRANDE, |r| r.size().max(TAILLE_MIN_GRANDE));
        commandes.push(ViewportCommand::InnerSize(taille));
        commandes.push(ViewportCommand::MinInnerSize(TAILLE_MIN_GRANDE));
        if let Some(grande) = self.grande {
            commandes.push(ViewportCommand::OuterPosition(grande.min));
        }
        commandes
    }

    /// Note où se trouve le mini-lecteur (on peut l'avoir déplacé).
    pub fn suivre_la_position(&mut self, ctx: &egui::Context) {
        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            self.position = Some(rect.min);
        }
    }

    /// Mémorise la place du mini-lecteur pour la prochaine fois.
    pub fn memoriser(&self) {
        if let Some(position) = self.position {
            memoriser(FICHIER, &ecrire_position(position));
        }
    }
}

/// « 120 640 » → la position (x, y). Rien si le texte est abîmé.
fn lire_position(texte: &str) -> Option<Pos2> {
    let mut nombres = texte.split_whitespace().map(str::parse::<f32>);
    let x = nombres.next()?.ok()?;
    let y = nombres.next()?.ok()?;
    (x.is_finite() && y.is_finite()).then(|| Pos2::new(x, y))
}

fn ecrire_position(position: Pos2) -> String {
    format!("{} {}", position.x.round(), position.y.round())
}

/// Dessine le mini-lecteur dans toute la fenêtre. Renvoie ce qu'on a demandé.
pub fn afficher(ctx: &egui::Context, affichage: &Affichage) -> Vec<Action> {
    let mut actions = Vec::new();
    let cadre = egui::Frame::none()
        .fill(theme::attenue(theme::PANNEAU, 235))
        .inner_margin(egui::Margin::same(10.0));
    egui::CentralPanel::default().frame(cadre).show(ctx, |ui| {
        // Sans barre de titre, on déplace la fenêtre en la faisant glisser
        // n'importe où (les boutons, ajoutés après, passent devant).
        let fond = ui.interact(
            ui.max_rect().expand(10.0),
            ui.id().with("deplacer"),
            egui::Sense::drag(),
        );
        if fond.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            dessiner_pochette(ui, affichage.pochette);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&affichage.titre)
                            .size(15.0)
                            .color(theme::TEXTE),
                    )
                    .truncate()
                    .selectable(false),
                );
                let artiste = affichage.artiste.as_deref().unwrap_or("");
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(artiste)
                            .size(12.5)
                            .color(theme::TEXTE_DISCRET),
                    )
                    .truncate()
                    .selectable(false),
                );
                ui.add_space(4.0);
                if let Some(fraction) = barre(ui, affichage) {
                    actions.push(Action::Sauter(fraction));
                }
                ui.add_space(4.0);
                boutons(ui, affichage, &mut actions);
            });
        });
    });
    // Un fin contour néon autour de la fenêtre, qui n'a pas de bord.
    let bord = ctx.screen_rect().shrink(0.5);
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        "bord_mini".into(),
    ))
    .rect_stroke(
        bord,
        0.0,
        egui::Stroke::new(1.0_f32, theme::attenue(theme::CYAN, 140)),
    );
    actions
}

fn dessiner_pochette(ui: &mut egui::Ui, pochette: Option<&egui::TextureHandle>) {
    let (cadre, _) = ui.allocate_exact_size(Vec2::splat(COTE_POCHETTE), egui::Sense::hover());
    let peintre = ui.painter_at(cadre);
    match pochette {
        Some(pochette) => {
            // L'image remplit le carré, coupée plutôt que déformée.
            let image = pochette.size_vec2();
            let echelle = (cadre.width() / image.x).max(cadre.height() / image.y);
            let visible = cadre.size() / (image * echelle);
            let uv = Rect::from_center_size(Pos2::new(0.5, 0.5), visible);
            peintre.image(pochette.id(), cadre, uv, egui::Color32::WHITE);
        }
        None => {
            // Pas de pochette : le fond nuit et ses deux taches de néon.
            peintre.rect_filled(cadre, 0.0, egui::Color32::from_rgb(0x0B, 0x07, 0x18));
            let points = theme::contour(cadre, Coins::AUCUN);
            let rien = egui::Color32::TRANSPARENT;
            let magenta = theme::attenue(theme::MAGENTA, 70);
            let cyan = theme::attenue(theme::CYAN, 60);
            theme::degrade(&peintre, &points, magenta, rien, egui::vec2(1.0, 1.0));
            theme::degrade(&peintre, &points, rien, cyan, egui::vec2(1.0, 1.0));
        }
    }
    peintre.rect_stroke(
        cadre,
        0.0,
        egui::Stroke::new(1.0_f32, theme::attenue(theme::CYAN, 85)),
    );
}

/// La fine barre de progression et le temps. Renvoie l'endroit cliqué.
fn barre(ui: &mut egui::Ui, affichage: &Affichage) -> Option<f32> {
    let mut saut = None;
    ui.horizontal(|ui| {
        let police = egui::FontId::monospace(11.0);
        let largeur_temps = 78.0;
        let largeur = (ui.available_width() - largeur_temps - 8.0).max(40.0);
        let sens = if affichage.peut_sauter {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };
        let (zone, reponse) = ui.allocate_exact_size(egui::vec2(largeur, 14.0), sens);
        let ligne = Rect::from_center_size(zone.center(), egui::vec2(zone.width(), 4.0));
        let peintre = ui.painter();
        peintre.rect_filled(ligne, 2.0, theme::attenue(theme::CYAN, 40));
        let fraction = affichage.fraction.clamp(0.0, 1.0);
        let x = ligne.left() + ligne.width() * fraction;
        let faite = Rect::from_min_max(ligne.min, egui::pos2(x, ligne.bottom()));
        peintre.rect_filled(faite, 2.0, theme::CYAN);
        peintre.circle_filled(egui::pos2(x, ligne.center().y), 4.5, theme::CYAN);
        if let Some(souris) = reponse.interact_pointer_pos().filter(|_| reponse.clicked()) {
            saut = Some(fraction_sous(ligne, souris.x));
        }
        ui.add_sized(
            egui::vec2(largeur_temps, 14.0),
            egui::Label::new(
                egui::RichText::new(&affichage.temps)
                    .font(police)
                    .color(theme::TEXTE_DISCRET),
            )
            .selectable(false),
        );
    });
    saut
}

/// La fraction du morceau sous la souris, sur la barre.
fn fraction_sous(ligne: Rect, x: f32) -> f32 {
    if ligne.width() <= 0.0 {
        return 0.0;
    }
    ((x - ligne.left()) / ligne.width()).clamp(0.0, 1.0)
}

fn boutons(ui: &mut egui::Ui, affichage: &Affichage, actions: &mut Vec<Action>) {
    let carre = egui::vec2(34.0, 34.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_enabled_ui(affichage.boutons_actifs, |ui| {
            let bouton = Bouton::icone(Icone::Precedent)
                .coins(Coins::haut_gauche(8.0))
                .taille(carre);
            if ui.add(bouton).on_hover_text("Morceau précédent").clicked() {
                actions.push(Action::Precedent);
            }
            let (icone, aide) = if affichage.en_lecture {
                (Icone::Pause, "Pause")
            } else {
                (Icone::Lecture, "Lecture")
            };
            let bouton = Bouton::icone(icone)
                .allure(Allure::Hexagone)
                .taille(egui::vec2(40.0, 40.0));
            if ui.add(bouton).on_hover_text(aide).clicked() {
                actions.push(Action::LecturePause);
            }
            let bouton = Bouton::icone(Icone::Suivant)
                .coins(Coins::bas_droit(8.0))
                .taille(carre);
            if ui.add(bouton).on_hover_text("Morceau suivant").clicked() {
                actions.push(Action::Suivant);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let bouton = Bouton::texte("Agrandir")
                .petit()
                .allure(Allure::Eteint)
                .taille(egui::vec2(92.0, 26.0));
            if ui
                .add(bouton)
                .on_hover_text("Revenir au lecteur en grand")
                .clicked()
            {
                actions.push(Action::Agrandir);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_position_se_relit_comme_on_l_a_ecrite() {
        let position = Pos2::new(120.0, -640.0);
        assert_eq!(lire_position(&ecrire_position(position)), Some(position));
        assert_eq!(lire_position(" 10   20\n"), Some(Pos2::new(10.0, 20.0)));
    }

    #[test]
    fn une_position_abimee_est_ignoree() {
        assert_eq!(lire_position(""), None);
        assert_eq!(lire_position("12"), None);
        assert_eq!(lire_position("a b"), None);
        assert_eq!(lire_position("NaN 3"), None);
    }

    #[test]
    fn en_mini_la_fenetre_rapetisse_et_reste_au_dessus() {
        let mini = MiniLecteur::default();
        let commandes = mini.commandes_pour_entrer();
        let rang = |c: &ViewportCommand| commandes.iter().position(|x| x == c).unwrap();
        assert!(commandes.contains(&ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop
        )));
        assert!(commandes.contains(&ViewportCommand::Decorations(false)));
        // La taille minimale baisse avant qu'on rapetisse la fenêtre.
        assert!(
            rang(&ViewportCommand::MinInnerSize(TAILLE))
                < rang(&ViewportCommand::InnerSize(TAILLE))
        );
        // Pas de place mémorisée : on laisse la fenêtre où elle est.
        assert!(!commandes
            .iter()
            .any(|c| matches!(c, ViewportCommand::OuterPosition(_))));
    }

    #[test]
    fn le_mini_lecteur_revient_a_sa_place_memorisee() {
        let mini = MiniLecteur {
            position: Some(Pos2::new(50.0, 60.0)),
            ..Default::default()
        };
        assert!(mini
            .commandes_pour_entrer()
            .contains(&ViewportCommand::OuterPosition(Pos2::new(50.0, 60.0))));
    }

    #[test]
    fn en_agrandissant_on_retrouve_la_grande_fenetre_d_avant() {
        let grande = Rect::from_min_size(Pos2::new(100.0, 80.0), egui::vec2(1320.0, 900.0));
        let mini = MiniLecteur {
            actif: true,
            grande: Some(grande),
            position: None,
        };
        let commandes = mini.commandes_pour_sortir();
        assert!(commandes.contains(&ViewportCommand::WindowLevel(egui::WindowLevel::Normal)));
        assert!(commandes.contains(&ViewportCommand::Decorations(true)));
        assert!(commandes.contains(&ViewportCommand::InnerSize(egui::vec2(1320.0, 900.0))));
        assert!(commandes.contains(&ViewportCommand::OuterPosition(Pos2::new(100.0, 80.0))));
        assert!(commandes.contains(&ViewportCommand::MinInnerSize(TAILLE_MIN_GRANDE)));
    }

    #[test]
    fn la_grande_fenetre_ne_revient_jamais_trop_petite() {
        let petite = Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, 300.0));
        let mini = MiniLecteur {
            actif: true,
            grande: Some(petite),
            position: None,
        };
        assert!(mini
            .commandes_pour_sortir()
            .contains(&ViewportCommand::InnerSize(TAILLE_MIN_GRANDE)));
    }

    #[test]
    fn un_clic_sur_la_barre_donne_l_endroit_du_morceau() {
        let ligne = Rect::from_min_size(Pos2::new(100.0, 0.0), egui::vec2(200.0, 4.0));
        assert_eq!(fraction_sous(ligne, 200.0), 0.5);
        assert_eq!(fraction_sous(ligne, 50.0), 0.0);
        assert_eq!(fraction_sous(ligne, 400.0), 1.0);
    }
}
