//! Ce qui n'existe que sous Linux : l'icône du lecteur près de l'horloge.
//!
//! Elle suit la norme « StatusNotifierItem » des bureaux Linux (KDE,
//! XFCE, Cinnamon… et GNOME avec l'extension AppIndicator, installée
//! d'office sur Ubuntu), grâce à la bibliothèque ksni, qui parle au bureau
//! par D-Bus sans avoir besoin de GTK.
//!
//! Comme sous Windows : la croix (ou Alt+F4) range le lecteur et la musique
//! continue. Un clic sur l'icône fait revenir la fenêtre ; un clic droit
//! ouvre le même menu que sous Windows. Pour fermer vraiment le lecteur, il
//! y a « Quitter » dans ce menu, ou le bouton « Quitter » de la fenêtre.
//!
//! Différence avec Windows : Linux ne permet pas de retirer une fenêtre de
//! la barre des tâches. « Ranger » veut donc dire réduire la fenêtre.
//!
//! Si le bureau n'a pas de place pour les icônes (GNOME sans l'extension,
//! par exemple), il n'y a pas d'icône : la croix ferme le lecteur, comme
//! d'habitude. Si la place disparaît pendant que le lecteur est rangé, la
//! fenêtre revient toute seule (sinon on ne pourrait plus la retrouver).
//!
//! L'icône répond aux clics dans son propre fil (celui de ksni) : elle met
//! les clics de côté et réveille la fenêtre, qui les traite à sa prochaine
//! image.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;

use super::{icone_fenetre, icone_petite, texte_d_infobulle};
use crate::raccourcis::Commande;

/// Tous les combien le lecteur est réveillé tant qu'il est rangé, pour qu'il
/// passe bien au morceau suivant même sans être affiché.
const REVEIL: Duration = Duration::from_millis(250);

/// Un clic sur l'icône (ou dans son menu).
#[derive(Debug, PartialEq)]
enum Clic {
    Afficher,
    Commande(Commande),
    Quitter,
}

/// Ce que la fenêtre, l'icône et le fil de réveil partagent.
#[derive(Default)]
struct Partage {
    /// Vrai tant que le bureau affiche les icônes.
    en_ligne: AtomicBool,
    /// Vrai tant que la fenêtre est réduite : le fil de réveil la réveille.
    endormie: AtomicBool,
    /// Vrai quand le lecteur se ferme : le fil de réveil s'arrête.
    fin: AtomicBool,
}

/// L'icône telle que ksni la montre au bureau.
struct Icone {
    clics: Sender<Clic>,
    ctx: egui::Context,
    partage: Arc<Partage>,
    infobulle: String,
    images: Vec<ksni::Icon>,
}

impl Icone {
    fn envoyer(&self, clic: Clic) {
        let _ = self.clics.send(clic);
        self.ctx.request_repaint();
    }
}

impl ksni::Tray for Icone {
    fn id(&self) -> String {
        "dollwave".into()
    }

    fn title(&self) -> String {
        "Doll//Wave".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.images.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.infobulle.clone(),
            ..Default::default()
        }
    }

    /// Clic gauche : la fenêtre revient.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.envoyer(Clic::Afficher);
    }

    /// Clic droit : le même menu que sous Windows.
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let entree = |texte: &str, clic: fn() -> Clic| {
            StandardItem {
                label: texte.into(),
                activate: Box::new(move |icone: &mut Self| icone.envoyer(clic())),
                ..Default::default()
            }
            .into()
        };
        vec![
            entree("Afficher le lecteur", || Clic::Afficher),
            ksni::MenuItem::Separator,
            entree("Lecture / pause", || Clic::Commande(Commande::LecturePause)),
            entree("Morceau précédent", || Clic::Commande(Commande::Precedent)),
            entree("Morceau suivant", || Clic::Commande(Commande::Suivant)),
            ksni::MenuItem::Separator,
            entree("Quitter", || Clic::Quitter),
        ]
    }

    fn watcher_online(&self) {
        self.partage.en_ligne.store(true, Ordering::Relaxed);
        self.ctx.request_repaint();
    }

    /// Le bureau n'affiche plus les icônes : on continue d'attendre qu'il
    /// revienne, et la fenêtre ressort si elle était rangée.
    fn watcher_offline(&self, _raison: ksni::OfflineReason) -> bool {
        self.partage.en_ligne.store(false, Ordering::Relaxed);
        self.ctx.request_repaint();
        true
    }
}

/// L'icône près de l'horloge, et l'état de la fenêtre (rangée ou non).
pub struct ZoneDeNotification {
    /// Rien si le bureau n'affiche pas d'icônes : la croix ferme alors
    /// le lecteur.
    icone: Option<Handle<Icone>>,
    clics: Receiver<Clic>,
    partage: Arc<Partage>,
    reveil: Option<JoinHandle<()>>,
    infobulle: String,
    /// Vrai depuis que la croix a rangé la fenêtre, jusqu'à son retour.
    rangee: bool,
    /// Le lecteur a demandé à revenir au premier plan (MP3 ouvert depuis
    /// le gestionnaire de fichiers), comme un clic sur l'icône.
    a_afficher: bool,
    /// On a demandé à quitter vraiment : la fermeture n'est plus retenue.
    quitter: bool,
}

impl ZoneDeNotification {
    pub fn new(ctx: &egui::Context) -> Self {
        let (envoi, clics) = mpsc::channel();
        let partage = Arc::new(Partage::default());
        let mut zone = Self {
            icone: None,
            clics,
            partage: partage.clone(),
            reveil: None,
            infobulle: texte_d_infobulle(None),
            rangee: false,
            a_afficher: false,
            quitter: false,
        };
        // Pendant les tests, plusieurs lecteurs sont créés en même temps :
        // pas d'icône.
        if cfg!(test) {
            return zone;
        }
        let icone = Icone {
            clics: envoi,
            ctx: ctx.clone(),
            partage: partage.clone(),
            infobulle: zone.infobulle.clone(),
            images: images_de_l_icone(),
        };
        // Refusé si le bureau n'a pas de place pour les icônes.
        let Ok(icone) = icone.spawn() else {
            return zone;
        };
        partage.en_ligne.store(true, Ordering::Relaxed);
        zone.icone = Some(icone);
        zone.reveil = std::thread::Builder::new()
            .name("réveil du lecteur rangé".into())
            .spawn({
                let ctx = ctx.clone();
                move || {
                    while !partage.fin.load(Ordering::Relaxed) {
                        std::thread::sleep(REVEIL);
                        if partage.endormie.load(Ordering::Relaxed) {
                            ctx.request_repaint();
                        }
                    }
                }
            })
            .ok();
        zone
    }

    /// Ferme vraiment le lecteur (et pas seulement sa fenêtre).
    pub fn quitter(&mut self, ctx: &egui::Context) {
        self.quitter = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Ramène la fenêtre au premier plan, même rangée.
    pub fn afficher(&mut self, ctx: &egui::Context) {
        self.a_afficher = true;
        ctx.request_repaint();
    }

    /// À appeler à chaque image : range la fenêtre quand on vient de la
    /// fermer, la fait revenir quand on clique sur l'icône, et rend les
    /// commandes choisies dans le menu.
    pub fn surveiller(&mut self, ctx: &egui::Context, _frame: &eframe::Frame) -> Vec<Commande> {
        let mut commandes = Vec::new();
        let mut afficher = std::mem::take(&mut self.a_afficher);
        while let Ok(clic) = self.clics.try_recv() {
            match clic {
                Clic::Afficher => afficher = true,
                Clic::Commande(commande) => commandes.push(commande),
                Clic::Quitter => self.quitter(ctx),
            }
        }

        let en_ligne = self.icone.is_some() && self.partage.en_ligne.load(Ordering::Relaxed);
        // Rangée mais l'icône a disparu : on ne pourrait plus la retrouver.
        if self.rangee && !en_ligne {
            afficher = true;
        }

        // Fermer la fenêtre la range (la réduit), sauf si on a demandé à
        // quitter ou s'il n'y a pas d'icône pour la faire revenir.
        let fermer = ctx.input(|i| i.viewport().close_requested());
        if fermer && !self.quitter && en_ligne {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.rangee = true;
            afficher = false;
        }

        let reduite = ctx.input(|i| i.viewport().minimized == Some(true));
        if afficher {
            if reduite || self.rangee {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.rangee = false;
        }
        self.partage
            .endormie
            .store(reduite || self.rangee, Ordering::Relaxed);
        commandes
    }

    /// Le texte qui s'affiche quand la souris passe sur l'icône.
    pub fn infobulle(&mut self, morceau: Option<&str>) {
        let texte = texte_d_infobulle(morceau);
        if texte == self.infobulle {
            return;
        }
        if let Some(icone) = &self.icone {
            let nouveau = texte.clone();
            icone.update(move |icone| icone.infobulle = nouveau);
        }
        self.infobulle = texte;
    }
}

impl Drop for ZoneDeNotification {
    /// À la fermeture du lecteur, on retire l'icône et on arrête le réveil.
    fn drop(&mut self) {
        self.partage.fin.store(true, Ordering::Relaxed);
        if let Some(icone) = self.icone.take() {
            icone.shutdown().wait();
        }
        if let Some(reveil) = self.reveil.take() {
            let _ = reveil.join();
        }
    }
}

/// L'icône en 32 et 256 pixels : le bureau prend la taille qui lui va.
fn images_de_l_icone() -> Vec<ksni::Icon> {
    let mut images = Vec::new();
    if let Some((pixels, largeur, hauteur)) = icone_petite() {
        images.push(image_ksni(&pixels, largeur, hauteur));
    }
    let grande = icone_fenetre();
    if grande.width > 0 {
        images.push(image_ksni(&grande.rgba, grande.width, grande.height));
    }
    images
}

/// Le bureau attend les pixels dans l'ordre opacité, rouge, vert, bleu
/// (et non rouge, vert, bleu, opacité comme dans nos PNG).
fn image_ksni(rgba: &[u8], largeur: u32, hauteur: u32) -> ksni::Icon {
    let data = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[3], p[0], p[1], p[2]])
        .collect();
    ksni::Icon {
        width: largeur as i32,
        height: hauteur as i32,
        data,
    }
}

/// Sous Linux, le lecteur passe par X11 (directement, ou par XWayland sur
/// un bureau Wayland) : Wayland ne laisse pas une fenêtre réduite revenir
/// au premier plan d'un clic sur l'icône, et arrête de la faire vivre tant
/// qu'elle est réduite (la liste de lecture s'arrêterait au morceau en
/// cours). S'il n'y a pas d'X11 du tout, on laisse le choix par défaut.
pub fn choisir_l_affichage(options: &mut eframe::NativeOptions) {
    if std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty()) {
        return;
    }
    options.event_loop_builder = Some(Box::new(|constructeur| {
        use winit::platform::x11::EventLoopBuilderExtX11;
        constructeur.with_x11();
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_dans_l_ordre_du_bureau() {
        let icone = image_ksni(&[1, 2, 3, 4, 5, 6, 7, 8], 2, 1);
        assert_eq!(icone.data, vec![4, 1, 2, 3, 8, 5, 6, 7]);
        assert_eq!((icone.width, icone.height), (2, 1));
    }

    #[test]
    fn l_icone_existe_en_deux_tailles() {
        let images = images_de_l_icone();
        let tailles: Vec<_> = images.iter().map(|i| (i.width, i.height)).collect();
        assert_eq!(tailles, vec![(32, 32), (256, 256)]);
        assert!(images.iter().all(|i| i.data.len() == (i.width * i.height * 4) as usize));
    }

    #[test]
    fn le_menu_comme_sous_windows() {
        let (envoi, clics) = mpsc::channel();
        let mut icone = Icone {
            clics: envoi,
            ctx: egui::Context::default(),
            partage: Arc::default(),
            infobulle: String::new(),
            images: Vec::new(),
        };
        let menu = ksni::Tray::menu(&icone);
        let textes: Vec<_> = menu
            .iter()
            .map(|entree| match entree {
                ksni::MenuItem::Standard(e) => e.label.as_str(),
                _ => "—",
            })
            .collect();
        assert_eq!(
            textes,
            [
                "Afficher le lecteur",
                "—",
                "Lecture / pause",
                "Morceau précédent",
                "Morceau suivant",
                "—",
                "Quitter"
            ]
        );
        for entree in &menu {
            if let ksni::MenuItem::Standard(e) = entree {
                (e.activate)(&mut icone);
            }
        }
        let recus: Vec<_> = clics.try_iter().collect();
        assert_eq!(
            recus,
            [
                Clic::Afficher,
                Clic::Commande(Commande::LecturePause),
                Clic::Commande(Commande::Precedent),
                Clic::Commande(Commande::Suivant),
                Clic::Quitter
            ]
        );
    }
}
