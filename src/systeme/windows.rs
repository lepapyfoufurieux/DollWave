//! Ce qui n'existe que sous Windows : l'icône du lecteur près de l'horloge.
//!
//! Quand on réduit la fenêtre, elle quitte la barre des tâches et le
//! lecteur n'est plus qu'une petite icône près de l'horloge. Un clic sur
//! l'icône fait revenir la fenêtre ; un clic droit ouvre un menu (lecture,
//! suivant, quitter…).
//!
//! La fenêtre reste « réduite » et non pas cachée : Windows continue ainsi
//! de la faire vivre, et le lecteur passe toujours au morceau suivant. Pour
//! qu'elle disparaisse de la barre des tâches, on la marque comme une
//! « fenêtre d'outil » (comme les petites palettes), le temps qu'elle reste
//! rangée.
//!
//! Les clics sur l'icône et dans son menu arrivent par Windows, parfois au
//! beau milieu du travail de la fenêtre : on se contente alors de les mettre
//! de côté, sans toucher à egui (sinon le lecteur se fige), et la fenêtre
//! les traite à sa prochaine image. Tant qu'elle est rangée, un petit fil la
//! réveille régulièrement pour cela (et pour passer au morceau suivant).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, SW_HIDE,
    SW_RESTORE, SW_SHOWMINNOACTIVE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use super::{icone_vinyle, texte_d_infobulle, TAILLE_ICONE};
use crate::raccourcis::Commande;

/// TRACES TEMPORAIRES pour trouver le bug du menu (à retirer avant le merge).
macro_rules! trace {
    ($($t:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!("[zone {:?}] {}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() % 100_000,
                format!($($t)*));
        }
    };
}

/// Tous les combien on réveille la fenêtre quand elle est rangée.
const REVEIL: Duration = Duration::from_millis(250);

/// Un clic sur l'icône (ou dans son menu).
enum Clic {
    Icone,
    Menu(MenuId),
}

/// Les entrées du menu du clic droit.
struct Entrees {
    afficher: MenuItem,
    lecture: MenuItem,
    precedent: MenuItem,
    suivant: MenuItem,
    quitter: MenuItem,
}

/// L'icône près de l'horloge, et l'état de la fenêtre (rangée ou non).
pub struct ZoneDeNotification {
    /// Rien si Windows a refusé de créer l'icône : on réduit alors la
    /// fenêtre normalement, dans la barre des tâches.
    icone: Option<(TrayIcon, Entrees)>,
    /// Le style de la fenêtre avant qu'on la range, pour le lui rendre.
    rangee: Option<isize>,
    /// Vrai tant que la fenêtre est rangée : le fil de réveil travaille.
    endormie: Arc<AtomicBool>,
    clics: Receiver<Clic>,
    infobulle: String,
}

impl ZoneDeNotification {
    pub fn new(ctx: &egui::Context) -> Self {
        let (envoi, clics) = mpsc::channel();
        // Les clics sont seulement mis de côté (voir en haut du fichier).
        let envoi_menu = envoi.clone();
        MenuEvent::set_event_handler(Some(move |evenement: MenuEvent| {
            trace!("menu cliqué : {:?}", evenement.id);
            let _ = envoi_menu.send(Clic::Menu(evenement.id));
        }));
        TrayIconEvent::set_event_handler(Some(move |evenement| {
            if !matches!(evenement, TrayIconEvent::Move { .. }) {
                trace!("icône : {evenement:?}");
            }
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = evenement
            {
                let _ = envoi.send(Clic::Icone);
            }
        }));

        let endormie = Arc::new(AtomicBool::new(false));
        let (reveil, ctx_reveil) = (endormie.clone(), ctx.clone());
        std::thread::spawn(move || loop {
            std::thread::sleep(REVEIL);
            if reveil.load(Ordering::Relaxed) {
                ctx_reveil.request_repaint();
            }
        });

        Self {
            icone: creer_l_icone(),
            rangee: None,
            endormie,
            clics,
            infobulle: String::new(),
        }
    }

    /// À appeler à chaque image : range la fenêtre près de l'horloge quand
    /// on vient de la réduire, la fait revenir quand on clique sur l'icône,
    /// et rend les commandes choisies dans le menu.
    pub fn surveiller(&mut self, ctx: &egui::Context, frame: &eframe::Frame) -> Vec<Commande> {
        let mut commandes = Vec::new();
        let Some((_, entrees)) = &self.icone else {
            return commandes;
        };
        let Some(fenetre) = fenetre(frame) else {
            trace!("pas de fenêtre Windows !");
            return commandes;
        };

        let mut afficher = false;
        if self.rangee.is_some() {
            trace!("image pendant que la fenêtre est rangée");
        }
        while let Ok(clic) = self.clics.try_recv() {
            trace!(
                "clic traité : {}",
                matches!(clic, Clic::Icone)
                    .then_some("icône")
                    .unwrap_or("menu")
            );
            let id = match clic {
                Clic::Icone => {
                    afficher = true;
                    continue;
                }
                Clic::Menu(id) => id,
            };
            if id == *entrees.afficher.id() {
                afficher = true;
            } else if id == *entrees.lecture.id() {
                commandes.push(Commande::LecturePause);
            } else if id == *entrees.precedent.id() {
                commandes.push(Commande::Precedent);
            } else if id == *entrees.suivant.id() {
                commandes.push(Commande::Suivant);
            } else if id == *entrees.quitter.id() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        let reduite = ctx.input(|i| i.viewport().minimized == Some(true));
        match self.rangee {
            Some(style) if afficher => {
                ressortir(fenetre, style);
                self.rangee = None;
            }
            // Revenue sans passer par l'icône (« Afficher le bureau » deux
            // fois de suite, par exemple) : on la remet dans la barre des tâches.
            Some(style) if !reduite => {
                ressortir(fenetre, style);
                self.rangee = None;
            }
            // Déjà visible : on la passe juste au premier plan.
            None if afficher => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
            None if reduite => self.rangee = Some(ranger(fenetre)),
            _ => {}
        }
        self.endormie
            .store(self.rangee.is_some(), Ordering::Relaxed);
        commandes
    }

    /// Le texte qui s'affiche quand la souris passe sur l'icône.
    pub fn infobulle(&mut self, morceau: Option<&str>) {
        let texte = texte_d_infobulle(morceau);
        if texte != self.infobulle {
            if let Some((icone, _)) = &self.icone {
                let _ = icone.set_tooltip(Some(&texte));
            }
            self.infobulle = texte;
        }
    }
}

fn creer_l_icone() -> Option<(TrayIcon, Entrees)> {
    let entrees = Entrees {
        afficher: MenuItem::new("Afficher le lecteur", true, None),
        lecture: MenuItem::new("Lecture / pause", true, None),
        precedent: MenuItem::new("Morceau précédent", true, None),
        suivant: MenuItem::new("Morceau suivant", true, None),
        quitter: MenuItem::new("Quitter", true, None),
    };
    let menu = Menu::with_items(&[
        &entrees.afficher,
        &PredefinedMenuItem::separator(),
        &entrees.lecture,
        &entrees.precedent,
        &entrees.suivant,
        &PredefinedMenuItem::separator(),
        &entrees.quitter,
    ])
    .ok()?;
    let image = Icon::from_rgba(icone_vinyle(TAILLE_ICONE), TAILLE_ICONE, TAILLE_ICONE).ok()?;
    let icone = TrayIconBuilder::new()
        .with_icon(image)
        .with_tooltip(texte_d_infobulle(None))
        .with_menu(Box::new(menu))
        // Clic gauche : la fenêtre revient ; clic droit : le menu.
        .with_menu_on_left_click(false)
        .build()
        .ok()?;
    Some((icone, entrees))
}

/// La fenêtre du lecteur, telle que Windows la connaît.
fn fenetre(frame: &eframe::Frame) -> Option<HWND> {
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(fenetre) => Some(fenetre.hwnd.get() as HWND),
        _ => None,
    }
}

/// Retire la fenêtre (déjà réduite) de la barre des tâches, et rend son
/// style d'origine.
fn ranger(fenetre: HWND) -> isize {
    trace!("ranger");
    // SAFETY : la fenêtre est celle du lecteur, ouverte pendant tout son
    // fonctionnement ; on ne change que son style et son affichage.
    unsafe {
        let style = GetWindowLongPtrW(fenetre, GWL_EXSTYLE);
        // Windows ne regarde le style qu'au moment où la fenêtre réapparaît.
        ShowWindow(fenetre, SW_HIDE);
        let outil = (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
        SetWindowLongPtrW(fenetre, GWL_EXSTYLE, outil);
        ShowWindow(fenetre, SW_SHOWMINNOACTIVE);
        style
    }
}

/// Remet la fenêtre dans la barre des tâches, à sa taille d'avant, au
/// premier plan.
fn ressortir(fenetre: HWND, style: isize) {
    trace!("ressortir");
    // SAFETY : voir `ranger`.
    unsafe {
        ShowWindow(fenetre, SW_HIDE);
        SetWindowLongPtrW(fenetre, GWL_EXSTYLE, style);
        ShowWindow(fenetre, SW_RESTORE);
        SetForegroundWindow(fenetre);
    }
}
