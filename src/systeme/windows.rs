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
//! L'icône vit dans son propre fil, avec sa propre boucle de messages
//! Windows. Le menu du clic droit bloque la boucle qui l'affiche le temps
//! qu'il est ouvert : dans la boucle de la fenêtre, il l'empêchait ensuite
//! de se redessiner, et le lecteur ne traitait plus aucun clic. Les clics
//! sont mis de côté par ce fil, qui réveille la fenêtre régulièrement tant
//! qu'elle est rangée pour qu'elle les traite (et passe au morceau suivant).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetWindowLongPtrW, MsgWaitForMultipleObjects, PeekMessageW,
    SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TranslateMessage, GWL_EXSTYLE, MSG,
    PM_REMOVE, QS_ALLINPUT, SW_HIDE, SW_RESTORE, SW_SHOWMINNOACTIVE, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW,
};

use super::{icone_vinyle, texte_d_infobulle, TAILLE_ICONE};
use crate::raccourcis::Commande;

/// Tous les combien (en millisecondes) le fil de l'icône regarde s'il a
/// quelque chose à faire, et réveille la fenêtre quand elle est rangée.
const REVEIL_MS: u32 = 250;

/// Un clic sur l'icône (ou dans son menu).
enum Clic {
    Icone,
    Menu(MenuId),
}

/// Les identifiants des entrées du menu du clic droit.
struct Entrees {
    afficher: MenuId,
    lecture: MenuId,
    precedent: MenuId,
    suivant: MenuId,
    quitter: MenuId,
}

/// Ce que la fenêtre et le fil de l'icône partagent.
#[derive(Default)]
struct Partage {
    /// Vrai tant que la fenêtre est rangée : le fil la réveille.
    endormie: AtomicBool,
    /// Vrai quand le lecteur se ferme : le fil retire l'icône et s'arrête.
    fin: AtomicBool,
    /// La nouvelle infobulle à afficher, s'il y en a une.
    infobulle: Mutex<Option<String>>,
}

/// L'icône près de l'horloge, et l'état de la fenêtre (rangée ou non).
pub struct ZoneDeNotification {
    /// Rien si Windows a refusé de créer l'icône : on réduit alors la
    /// fenêtre normalement, dans la barre des tâches.
    entrees: Option<Entrees>,
    /// Le style de la fenêtre avant qu'on la range, pour le lui rendre.
    rangee: Option<isize>,
    partage: Arc<Partage>,
    clics: Receiver<Clic>,
    fil: Option<JoinHandle<()>>,
    infobulle: String,
}

impl ZoneDeNotification {
    pub fn new(ctx: &egui::Context) -> Self {
        let (envoi, clics) = mpsc::channel();
        let partage = Arc::new(Partage::default());
        // Pendant les tests, plusieurs lecteurs sont créés en même temps,
        // chacun dans son fil : pas d'icône, car tray-icon n'a qu'un seul
        // gestionnaire de clics pour tout le programme (le test plante sinon).
        if cfg!(test) {
            drop(envoi);
            return Self {
                entrees: None,
                rangee: None,
                partage,
                clics,
                fil: None,
                infobulle: String::new(),
            };
        }
        let (envoi_entrees, entrees) = mpsc::channel();
        let fil = {
            let (partage, ctx) = (partage.clone(), ctx.clone());
            std::thread::Builder::new()
                .name("icône près de l'horloge".into())
                .spawn(move || fil_de_l_icone(envoi, envoi_entrees, partage, ctx))
                .ok()
        };
        Self {
            // Le fil dit tout de suite si l'icône a pu être créée.
            entrees: entrees.recv().ok().flatten(),
            rangee: None,
            partage,
            clics,
            fil,
            infobulle: String::new(),
        }
    }

    /// À appeler à chaque image : range la fenêtre près de l'horloge quand
    /// on vient de la réduire, la fait revenir quand on clique sur l'icône,
    /// et rend les commandes choisies dans le menu.
    pub fn surveiller(&mut self, ctx: &egui::Context, frame: &eframe::Frame) -> Vec<Commande> {
        let mut commandes = Vec::new();
        let Some(entrees) = &self.entrees else {
            return commandes;
        };
        let Some(fenetre) = fenetre(frame) else {
            return commandes;
        };

        let mut afficher = false;
        while let Ok(clic) = self.clics.try_recv() {
            let id = match clic {
                Clic::Icone => {
                    afficher = true;
                    continue;
                }
                Clic::Menu(id) => id,
            };
            if id == entrees.afficher {
                afficher = true;
            } else if id == entrees.lecture {
                commandes.push(Commande::LecturePause);
            } else if id == entrees.precedent {
                commandes.push(Commande::Precedent);
            } else if id == entrees.suivant {
                commandes.push(Commande::Suivant);
            } else if id == entrees.quitter {
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
        self.partage
            .endormie
            .store(self.rangee.is_some(), Ordering::Relaxed);
        commandes
    }

    /// Le texte qui s'affiche quand la souris passe sur l'icône.
    pub fn infobulle(&mut self, morceau: Option<&str>) {
        let texte = texte_d_infobulle(morceau);
        if texte != self.infobulle {
            if let Ok(mut infobulle) = self.partage.infobulle.lock() {
                *infobulle = Some(texte.clone());
            }
            self.infobulle = texte;
        }
    }
}

impl Drop for ZoneDeNotification {
    /// À la fermeture du lecteur, on attend que le fil ait retiré l'icône :
    /// sinon elle resterait près de l'horloge jusqu'au passage de la souris.
    fn drop(&mut self) {
        self.partage.fin.store(true, Ordering::Relaxed);
        if let Some(fil) = self.fil.take() {
            let _ = fil.join();
        }
    }
}

/// Le fil de l'icône : la crée, puis fait tourner sa boucle de messages
/// Windows (affichage du menu, clics…) jusqu'à la fermeture du lecteur.
fn fil_de_l_icone(
    envoi: Sender<Clic>,
    envoi_entrees: Sender<Option<Entrees>>,
    partage: Arc<Partage>,
    ctx: egui::Context,
) {
    let envoi_menu = envoi.clone();
    MenuEvent::set_event_handler(Some(move |evenement: MenuEvent| {
        let _ = envoi_menu.send(Clic::Menu(evenement.id));
    }));
    TrayIconEvent::set_event_handler(Some(move |evenement| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = evenement
        {
            let _ = envoi.send(Clic::Icone);
        }
    }));

    let Some((icone, entrees)) = creer_l_icone() else {
        let _ = envoi_entrees.send(None);
        return;
    };
    let _ = envoi_entrees.send(Some(entrees));

    while !partage.fin.load(Ordering::Relaxed) {
        // SAFETY : appels Windows classiques d'une boucle de messages, sur
        // ce fil-ci, avec un MSG à nous.
        unsafe {
            MsgWaitForMultipleObjects(0, std::ptr::null(), 0, REVEIL_MS, QS_ALLINPUT);
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let nouvelle = partage.infobulle.lock().ok().and_then(|mut i| i.take());
        if let Some(texte) = nouvelle {
            let _ = icone.set_tooltip(Some(texte));
        }
        if partage.endormie.load(Ordering::Relaxed) {
            ctx.request_repaint();
        }
    }
    // L'icône est retirée de la zone de notification ici, en la lâchant.
    drop(icone);
}

fn creer_l_icone() -> Option<(TrayIcon, Entrees)> {
    let afficher = MenuItem::new("Afficher le lecteur", true, None);
    let lecture = MenuItem::new("Lecture / pause", true, None);
    let precedent = MenuItem::new("Morceau précédent", true, None);
    let suivant = MenuItem::new("Morceau suivant", true, None);
    let quitter = MenuItem::new("Quitter", true, None);
    let menu = Menu::with_items(&[
        &afficher,
        &PredefinedMenuItem::separator(),
        &lecture,
        &precedent,
        &suivant,
        &PredefinedMenuItem::separator(),
        &quitter,
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
    let entrees = Entrees {
        afficher: afficher.id().clone(),
        lecture: lecture.id().clone(),
        precedent: precedent.id().clone(),
        suivant: suivant.id().clone(),
        quitter: quitter.id().clone(),
    };
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
    // SAFETY : voir `ranger`.
    unsafe {
        ShowWindow(fenetre, SW_HIDE);
        SetWindowLongPtrW(fenetre, GWL_EXSTYLE, style);
        ShowWindow(fenetre, SW_RESTORE);
        SetForegroundWindow(fenetre);
    }
}
