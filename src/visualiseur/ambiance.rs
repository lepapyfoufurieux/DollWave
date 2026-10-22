//! Le style « Ambiance », inspiré des visualiseurs de Windows Media Player
//! sous XP (Ambience, Battery) : une image qui remplit tout le cadre, où les
//! couleurs fondent, tourbillonnent et zooment en laissant une fumée.
//!
//! Le principe est celui des visualiseurs de l'époque : chaque nouvelle image
//! reprend la précédente un peu zoomée, tournée et estompée, puis la musique
//! dessine par-dessus (la vibration du son, des taches sur les graves, des
//! étincelles sur les aigus). L'image est petite et calculée pixel par pixel,
//! puis agrandie et adoucie par la carte graphique.

use std::f32::consts::TAU;

use eframe::egui;

use crate::theme;

/// Taille de l'image calculée, en pixels (elle est agrandie au dessin).
const LARGEUR: usize = 320;
const HAUTEUR: usize = 80;
/// Part de lumière gardée d'une image à l'autre (à 60 images par seconde) :
/// plus c'est proche de 1, plus la fumée traîne.
const MEMOIRE: f32 = 0.94;
/// Vitesse à laquelle les graves, médiums et aigus retombent (par seconde).
const CHUTE: f32 = 1.2;
/// Hauteur de barre en dessous de laquelle une bande compte pour rien :
/// sans ce seuil, le moindre bruit de fond ferait déjà tout bouger.
const SEUIL: f32 = 0.25;
/// En dessous de cette lumière, un pixel est considéré comme noir.
const NOIR: f32 = 0.004;

/// Une couleur en lumière (chaque canal entre 0 et 1, ou plus quand ça brille).
type Lumiere = [f32; 3];

pub struct Ambiance {
    /// Le temps de l'animation : il avance plus vite quand la musique est
    /// chargée, et s'arrête en pause.
    temps: f32,
    /// La force actuelle de chaque bande (entre 0 et 1).
    graves: f32,
    mediums: f32,
    aigus: f32,
    /// L'image actuelle et un brouillon de même taille, échangés à chaque image.
    image: Vec<Lumiere>,
    brouillon: Vec<Lumiere>,
    /// La même image, envoyée à la carte graphique pour être dessinée.
    texture: Option<egui::TextureHandle>,
}

impl Ambiance {
    pub fn new() -> Self {
        Self {
            temps: 0.0,
            graves: 0.0,
            mediums: 0.0,
            aigus: 0.0,
            image: vec![[0.0; 3]; LARGEUR * HAUTEUR],
            brouillon: vec![[0.0; 3]; LARGEUR * HAUTEUR],
            texture: None,
        }
    }

    /// Vrai quand plus rien ne bouge : la musique s'est tue et la fumée
    /// s'est éteinte.
    pub fn au_repos(&self) -> bool {
        let silence = self.graves <= 0.0 && self.mediums <= 0.0 && self.aigus <= 0.0;
        silence && self.image.iter().all(|p| p.iter().all(|&c| c < NOIR))
    }

    /// Calcule l'image suivante. `barres` : la force de chaque bande de
    /// fréquences (entre 0 et 1), des graves aux aigus ; `onde` : la vibration
    /// du son en ce moment (entre -1 et 1). Tout est à zéro en pause.
    /// `dt` : la durée de l'image, en secondes.
    pub fn avancer(&mut self, barres: &[f32], onde: &[f32], en_lecture: bool, dt: f32) {
        let n = barres.len();
        let force = |debut: usize, fin: usize| {
            let moyenne = barres[debut..fin].iter().sum::<f32>() / (fin - debut) as f32;
            ((moyenne - SEUIL) / (1.0 - SEUIL)).clamp(0.0, 1.0)
        };
        let cibles = [
            force(0, n / 5),
            force(n / 5, 2 * n / 3),
            force(2 * n / 3, n),
        ];
        // Comme les barres : on monte tout de suite, on redescend doucement.
        for (valeur, cible) in [&mut self.graves, &mut self.mediums, &mut self.aigus]
            .into_iter()
            .zip(cibles)
        {
            *valeur = cible.max(*valeur - CHUTE * dt);
        }
        // Le nombre d'images « à 60 par seconde » que représente celle-ci.
        let pas = (dt * 60.0).clamp(0.0, 4.0);
        if en_lecture {
            self.temps += dt * (0.4 + 1.0 * self.mediums + 0.6 * self.aigus);
        }
        self.fumee(pas);
        if en_lecture {
            self.dessiner_la_musique(onde);
        }
    }

    /// L'image précédente, zoomée, tournée, tordue en tourbillon et estompée.
    fn fumee(&mut self, pas: f32) {
        let t = self.temps;
        // Le zoom (vers l'extérieur) s'accélère sur les graves. La rotation
        // s'accélère sur les médiums et change de sens de temps en temps, en
        // ralentissant d'abord.
        let zoom = 1.0 - pas * (0.012 + 0.03 * self.graves);
        let sens = (3.0 * (0.23 * t).sin()).clamp(-1.0, 1.0);
        let rotation = pas * (0.012 + 0.025 * self.mediums) * sens;
        let tourbillon = pas * 0.02 * (0.5 + self.mediums);
        let estompe = MEMOIRE.powf(pas);
        let (cx, cy) = (LARGEUR as f32 / 2.0, HAUTEUR as f32 / 2.0);
        let echelle = HAUTEUR as f32;
        for y in 0..HAUTEUR {
            for x in 0..LARGEUR {
                // Position par rapport au centre, en « hauteurs d'image ».
                let u = (x as f32 + 0.5 - cx) / echelle;
                let v = (y as f32 + 0.5 - cy) / echelle;
                let rayon = (u * u + v * v).sqrt();
                let angle = rotation + tourbillon * (6.0 * rayon - 1.7 * t).sin();
                let (s, c) = angle.sin_cos();
                let su = (u * c - v * s) * zoom;
                let sv = (u * s + v * c) * zoom;
                let source = lire(
                    &self.image,
                    su * echelle + cx - 0.5,
                    sv * echelle + cy - 0.5,
                );
                self.brouillon[y * LARGEUR + x] = source.map(|canal| canal * estompe);
            }
        }
        std::mem::swap(&mut self.image, &mut self.brouillon);
    }

    /// La musique dessine par-dessus la fumée.
    fn dessiner_la_musique(&mut self, onde: &[f32]) {
        let t = self.temps;
        let (cx, cy) = (LARGEUR as f32 / 2.0, HAUTEUR as f32 / 2.0);

        // La vibration du son, en travers de toute l'image, d'une couleur qui
        // passe lentement du cyan au magenta puis au jaune.
        if !onde.is_empty() {
            let couleur = teinte(0.07 * t).map(|c| c * (0.2 + 0.35 * self.mediums));
            let ampleur = HAUTEUR as f32 * (0.25 + 0.2 * self.mediums);
            let mut avant: Option<f32> = None;
            for x in 0..LARGEUR {
                let e = onde[x * onde.len() / LARGEUR].clamp(-1.0, 1.0);
                let y = cy - e * ampleur;
                // On relie au point précédent pour que la ligne n'ait pas de trou.
                let (haut, bas) = match avant {
                    Some(p) => (p.min(y), p.max(y)),
                    None => (y, y),
                };
                let mut yy = haut;
                while yy <= bas {
                    tache(&mut self.image, x as f32, yy, 1.2, couleur);
                    yy += 1.0;
                }
                avant = Some(y);
            }
        }

        // Sur les graves : des taches qui tournent autour du centre.
        if self.graves > 0.05 {
            let couleur = teinte(0.07 * t + 0.33).map(|c| c * 0.6 * self.graves);
            let rayon = 3.0 + 7.0 * self.graves;
            for k in 0..3 {
                let a = 0.9 * t + TAU * k as f32 / 3.0;
                let x = cx + 1.6 * HAUTEUR as f32 * 0.3 * a.cos();
                let y = cy + HAUTEUR as f32 * 0.3 * a.sin();
                tache(&mut self.image, x, y, rayon, couleur);
            }
        }

        // Sur les aigus : des étincelles, à des places qui changent sans cesse.
        if self.aigus > 0.05 {
            let couleur = teinte(0.07 * t + 0.66).map(|c| c * 0.9 * self.aigus);
            let graine = (t * 40.0) as u32;
            for k in 0..(2 + (6.0 * self.aigus) as u32) {
                let x = hasard(graine.wrapping_mul(31).wrapping_add(k * 2)) * LARGEUR as f32;
                let y = hasard(graine.wrapping_mul(31).wrapping_add(k * 2 + 1)) * HAUTEUR as f32;
                tache(&mut self.image, x, y, 1.0, couleur);
            }
        }
    }

    /// Dessine l'image dans `zone`, agrandie et adoucie.
    pub fn dessiner(&mut self, ctx: &egui::Context, peintre: &egui::Painter, zone: egui::Rect) {
        let image = egui::ColorImage {
            size: [LARGEUR, HAUTEUR],
            pixels: self.image.iter().map(|&p| en_couleur(p)).collect(),
        };
        let options = egui::TextureOptions::LINEAR;
        match &mut self.texture {
            Some(texture) => texture.set(image, options),
            None => self.texture = Some(ctx.load_texture("ambiance", image, options)),
        }
        if let Some(texture) = &self.texture {
            let tout = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
            peintre.image(texture.id(), zone, tout, egui::Color32::WHITE);
        }
    }
}

/// La lumière de l'image au point (x, y), en mélangeant les quatre pixels
/// voisins. Hors de l'image : du noir.
fn lire(image: &[Lumiere], x: f32, y: f32) -> Lumiere {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let pixel = |px: f32, py: f32| -> Lumiere {
        if px < 0.0 || py < 0.0 || px >= LARGEUR as f32 || py >= HAUTEUR as f32 {
            [0.0; 3]
        } else {
            image[py as usize * LARGEUR + px as usize]
        }
    };
    let (a, b) = (pixel(x0, y0), pixel(x0 + 1.0, y0));
    let (c, d) = (pixel(x0, y0 + 1.0), pixel(x0 + 1.0, y0 + 1.0));
    std::array::from_fn(|i| {
        let haut = a[i] + (b[i] - a[i]) * fx;
        let bas = c[i] + (d[i] - c[i]) * fx;
        haut + (bas - haut) * fy
    })
}

/// Ajoute une tache de lumière ronde et floue autour de (x, y).
fn tache(image: &mut [Lumiere], x: f32, y: f32, rayon: f32, couleur: Lumiere) {
    let portee = (rayon * 2.0).ceil() as isize;
    let (px, py) = (x.round() as isize, y.round() as isize);
    for yy in (py - portee).max(0)..(py + portee + 1).min(HAUTEUR as isize) {
        for xx in (px - portee).max(0)..(px + portee + 1).min(LARGEUR as isize) {
            let dx = xx as f32 + 0.5 - x;
            let dy = yy as f32 + 0.5 - y;
            let poids = (-(dx * dx + dy * dy) / (rayon * rayon)).exp();
            let pixel = &mut image[yy as usize * LARGEUR + xx as usize];
            for (canal, ajout) in pixel.iter_mut().zip(couleur) {
                *canal += ajout * poids;
            }
        }
    }
}

/// Une couleur de la palette néon qui tourne : cyan → magenta → jaune → cyan,
/// selon `phase` (un tour complet par unité).
fn teinte(phase: f32) -> Lumiere {
    let palette = [theme::CYAN, theme::MAGENTA, theme::JAUNE];
    let p = phase.rem_euclid(1.0) * 3.0;
    let i = p as usize % 3;
    let f = p.fract();
    let (a, b) = (palette[i], palette[(i + 1) % 3]);
    let canal = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f) / 255.0;
    [
        canal(a.r(), b.r()),
        canal(a.g(), b.g()),
        canal(a.b(), b.b()),
    ]
}

/// Transforme la lumière en couleur d'écran : les fortes lumières tendent
/// doucement vers le blanc au lieu d'être coupées net.
fn en_couleur(p: Lumiere) -> egui::Color32 {
    let canal = |c: f32| ((1.0 - (-1.3 * c).exp()) * 255.0) as u8;
    egui::Color32::from_rgb(canal(p[0]), canal(p[1]), canal(p[2]))
}

/// Un nombre entre 0 et 1 qui a l'air pris au hasard, mais toujours le même
/// pour la même graine.
fn hasard(graine: u32) -> f32 {
    let mut h = graine.wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    (h & 0xFFFF) as f32 / 65_536.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: f32 = 1.0 / 60.0;

    fn onde(images: usize) -> Vec<f32> {
        (0..1024)
            .map(|i| (TAU * i as f32 / 128.0 + images as f32 * 0.3).sin() * 0.6)
            .collect()
    }

    fn lumiere_totale(ambiance: &Ambiance) -> f32 {
        ambiance.image.iter().flatten().sum()
    }

    #[test]
    fn les_graves_font_reagir_les_graves_seulement() {
        let mut ambiance = Ambiance::new();
        let mut barres = [0.0; 32];
        barres[..6].fill(1.0);
        ambiance.avancer(&barres, &[], true, IMAGE);
        assert!(ambiance.graves > 0.9);
        assert!(ambiance.mediums < 0.1 && ambiance.aigus < 0.1);
    }

    #[test]
    fn la_musique_allume_l_image() {
        let mut ambiance = Ambiance::new();
        for i in 0..30 {
            ambiance.avancer(&[0.8; 32], &onde(i), true, IMAGE);
        }
        assert!(lumiere_totale(&ambiance) > 100.0);
    }

    #[test]
    fn en_pause_la_fumee_s_eteint_puis_tout_s_arrete() {
        let mut ambiance = Ambiance::new();
        for i in 0..60 {
            ambiance.avancer(&[1.0; 32], &onde(i), true, IMAGE);
        }
        assert!(!ambiance.au_repos());
        let temps = ambiance.temps;
        let mut images = 0;
        while !ambiance.au_repos() && images < 600 {
            ambiance.avancer(&[0.0; 32], &[], false, IMAGE);
            images += 1;
        }
        // Moins de 3 secondes pour que tout s'éteigne.
        assert!(images <= 180, "{images} images pour s'éteindre");
        // En pause, le tourbillon ne tourne plus.
        assert_eq!(ambiance.temps, temps);
    }

    #[test]
    fn la_palette_tourne_entre_les_trois_neons() {
        let cyan = teinte(0.0);
        assert!(cyan[2] > 0.99 && cyan[0] < 0.01);
        let magenta = teinte(1.0 / 3.0);
        assert!(magenta[0] > 0.99 && magenta[1] < 0.2);
        // Un tour complet ramène au cyan.
        assert_eq!(teinte(1.0), cyan);
    }
}
