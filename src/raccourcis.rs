//! Les raccourcis clavier (et la molette pour le volume).

use eframe::egui::{self, Key, Modifiers};

/// Ce qu'une touche demande au lecteur.
pub enum Commande {
    LecturePause,
    Stop,
    Precedent,
    Suivant,
    /// Se déplacer dans le morceau, en secondes (négatif = reculer).
    Deplacer(f32),
    Muet,
    Aleatoire,
    Repetition,
}

/// De combien Q et D font reculer ou avancer dans le morceau.
const SAUT: f32 = 5.0;

/// Les touches et leur commande. On compare la lettre écrite sur la touche,
/// donc Q et D sont bien les touches marquées Q et D sur un clavier AZERTY.
const TOUCHES: [(Key, Commande); 9] = [
    (Key::Space, Commande::LecturePause),
    (Key::S, Commande::Stop),
    (Key::ArrowLeft, Commande::Precedent),
    (Key::ArrowRight, Commande::Suivant),
    (Key::Q, Commande::Deplacer(-SAUT)),
    (Key::D, Commande::Deplacer(SAUT)),
    (Key::M, Commande::Muet),
    (Key::A, Commande::Aleatoire),
    (Key::R, Commande::Repetition),
];

/// La liste affichée dans l'aide « ⌨ Raccourcis ».
pub const AIDE: &str = "Espace : lecture / pause\n\
    S : stop\n\
    <- / -> : morceau précédent / suivant\n\
    Q / D : reculer / avancer de 5 secondes\n\
    M : couper / remettre le son\n\
    A : lecture aléatoire\n\
    R : lecture en boucle\n\
    Molette : volume";

/// Les commandes demandées au clavier depuis l'image précédente.
///
/// À appeler avant de dessiner l'interface : les touches utilisées sont
/// « consommées », pour qu'un bouton qui a le focus ne réagisse pas en plus
/// à Espace, et que les flèches ne servent pas à passer d'un bouton à l'autre.
pub fn lire(ctx: &egui::Context) -> Vec<Commande> {
    // Quand on écrit dans un champ texte (la recherche, par exemple), les
    // touches servent à écrire, pas à piloter le lecteur.
    // (On ne peut pas se fier à `wants_keyboard_input`, qui est vrai aussi
    // quand c'est un simple bouton qui a le focus.)
    let champ_texte = ctx
        .memory(|m| m.focused())
        .is_some_and(|id| egui::text_edit::TextEditState::load(ctx, id).is_some());
    if champ_texte {
        return Vec::new();
    }
    ctx.input_mut(|input| {
        let mut commandes = Vec::new();
        for (touche, commande) in TOUCHES {
            if input.consume_key(Modifiers::NONE, touche) {
                commandes.push(commande);
            }
        }
        commandes
    })
}

/// De combien changer le volume (de -1.0 à 1.0) selon la molette.
/// Un cran de molette fait environ 5 %.
pub fn molette(ctx: &egui::Context) -> f32 {
    // Sous Windows, un cran de molette vaut 50 « points » pour egui.
    ctx.input(|input| input.raw_scroll_delta.y) * 0.05 / 50.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simule l'appui sur une touche pendant une image, avec `dessiner` comme
    /// interface, et renvoie les commandes lues.
    fn appuyer(
        ctx: &egui::Context,
        touche: Key,
        dessiner: impl Fn(&mut egui::Ui),
    ) -> Vec<Commande> {
        let mut entree = egui::RawInput::default();
        entree.events.push(egui::Event::Key {
            key: touche,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        let mut commandes = Vec::new();
        let _ = ctx.run(entree, |ctx| {
            commandes = lire(ctx);
            egui::CentralPanel::default().show(ctx, |ui| dessiner(ui));
        });
        commandes
    }

    #[test]
    fn les_touches_donnent_leur_commande() {
        let ctx = egui::Context::default();
        let rien = |_: &mut egui::Ui| {};
        assert!(matches!(
            appuyer(&ctx, Key::Space, rien)[..],
            [Commande::LecturePause]
        ));
        assert!(matches!(
            appuyer(&ctx, Key::ArrowRight, rien)[..],
            [Commande::Suivant]
        ));
        assert!(matches!(appuyer(&ctx, Key::Q, rien)[..], [Commande::Deplacer(s)] if s < 0.0));
        assert!(matches!(
            appuyer(&ctx, Key::A, rien)[..],
            [Commande::Aleatoire]
        ));
        assert!(matches!(
            appuyer(&ctx, Key::R, rien)[..],
            [Commande::Repetition]
        ));
        assert!(appuyer(&ctx, Key::Z, rien).is_empty());
    }

    #[test]
    fn rien_pendant_qu_on_ecrit_dans_un_champ_texte() {
        let ctx = egui::Context::default();
        let champ = |ui: &mut egui::Ui| {
            let mut texte = String::new();
            let reponse = ui.text_edit_singleline(&mut texte);
            reponse.request_focus();
        };
        // Première image : le champ prend le focus.
        appuyer(&ctx, Key::Z, champ);
        assert!(appuyer(&ctx, Key::Space, champ).is_empty());
        assert!(appuyer(&ctx, Key::S, champ).is_empty());
    }

    #[test]
    fn espace_n_appuie_pas_en_plus_sur_le_bouton_qui_a_le_focus() {
        let ctx = egui::Context::default();
        let clics = std::cell::Cell::new(0);
        let bouton = |ui: &mut egui::Ui| {
            let reponse = ui.button("Stop");
            reponse.request_focus();
            if reponse.clicked() {
                clics.set(clics.get() + 1);
            }
        };
        appuyer(&ctx, Key::Z, bouton);
        assert!(matches!(
            appuyer(&ctx, Key::Space, bouton)[..],
            [Commande::LecturePause]
        ));
        assert_eq!(clics.get(), 0);
    }
}
