//! La boîte à outils de dessin du thème Néon : cadres à angles coupés,
//! dégradés, lueurs, hexagone, icônes et boutons néon (voir
//! design/interface-neon.md).
//!
//! Tout est dessiné avec les formes simples d'egui (polygones, traits,
//! maillages colorés) : rien à charger, et peu de calcul à chaque image.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use eframe::egui::{self, epaint, Color32, Pos2, Rect, Shape, Stroke, Vec2};

use super::{attenue, CYAN, FOND, PANNEAU, TEXTE_DISCRET};

/// La force des lueurs, de 0 (aucune) à 2 (très fortes). 1 par défaut ;
/// le réglage « Intensité des effets » la change (voir `fixer_intensite`).
static INTENSITE: AtomicU32 = AtomicU32::new(0x3F80_0000); // 1.0

/// La force des lueurs en ce moment (voir `INTENSITE`).
pub fn intensite() -> f32 {
    f32::from_bits(INTENSITE.load(Ordering::Relaxed))
}

/// Change la force des lueurs (gardée entre 0 et 2).
pub fn fixer_intensite(force: f32) {
    INTENSITE.store(force.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
}

/// La respiration des lueurs animées, de 0 (au plus faible) à 1 (au plus
/// fort). Elle bat pendant la lecture et reste à 1 sinon (écran figé).
static PULSATION: AtomicU32 = AtomicU32::new(0x3F80_0000); // 1.0

/// Les lignes d'écran (effet vieux moniteur) sont-elles affichées ?
static LIGNES: AtomicBool = AtomicBool::new(true);

/// La respiration des lueurs animées en ce moment (voir `PULSATION`).
pub fn pulsation() -> f32 {
    f32::from_bits(PULSATION.load(Ordering::Relaxed))
}

/// Change la respiration des lueurs (gardée entre 0 et 1).
pub fn fixer_pulsation(valeur: f32) {
    PULSATION.store(valeur.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

/// Affiche ou cache les lignes d'écran.
pub fn fixer_lignes(affichees: bool) {
    LIGNES.store(affichees, Ordering::Relaxed);
}

/// Des lignes d'écran sur `zone` : une ligne sombre d'un pixel toutes les
/// `ecart` points, d'opacité `opacite` (0 à 255) à l'intensité normale.
/// Rien si l'option est décochée ou si l'intensité des effets est à 0.
pub fn lignes_d_ecran(peintre: &egui::Painter, zone: Rect, ecart: f32, opacite: f32) {
    let alpha = (opacite * intensite()).round().min(255.0);
    if !LIGNES.load(Ordering::Relaxed) || alpha < 1.0 {
        return;
    }
    let trait_ = Stroke::new(1.0_f32, Color32::from_black_alpha(alpha as u8));
    let mut y = zone.top().floor() + 0.5;
    while y < zone.bottom() {
        peintre.hline(zone.x_range(), y, trait_);
        y += ecart;
    }
}

/// Une couleur dont l'opacité est multipliée par `facteur` (0 à 1).
pub fn fondu(couleur: Color32, facteur: f32) -> Color32 {
    couleur.gamma_multiply(facteur.clamp(0.0, 1.0))
}

/// La taille des angles coupés d'un cadre, dans l'ordre : en haut à gauche,
/// en haut à droite, en bas à droite, en bas à gauche (0 = angle droit).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coins(pub [f32; 4]);

impl Coins {
    pub const AUCUN: Coins = Coins([0.0; 4]);

    /// En haut à gauche et en bas à droite, comme les cadres de la maquette.
    pub fn diagonale(taille: f32) -> Self {
        Coins([taille, 0.0, taille, 0.0])
    }

    /// En haut à droite et en bas à gauche.
    pub fn autre_diagonale(taille: f32) -> Self {
        Coins([0.0, taille, 0.0, taille])
    }

    pub fn haut_gauche(taille: f32) -> Self {
        Coins([taille, 0.0, 0.0, 0.0])
    }

    pub fn haut_droit(taille: f32) -> Self {
        Coins([0.0, taille, 0.0, 0.0])
    }

    pub fn bas_droit(taille: f32) -> Self {
        Coins([0.0, 0.0, taille, 0.0])
    }

    /// Les mêmes angles, agrandis de `de` (pour une lueur autour du cadre).
    fn elargis(self, de: f32) -> Self {
        // Un angle coupé à 45° recule de (√2 − 1) quand on écarte ses côtés.
        Coins(self.0.map(|c| if c > 0.0 { c + de * 0.414 } else { 0.0 }))
    }
}

/// Le contour d'un rectangle aux angles coupés, dans le sens des aiguilles
/// d'une montre en partant du haut à gauche. Toujours convexe.
pub fn contour(rect: Rect, coins: Coins) -> Vec<Pos2> {
    let demi = rect.width().min(rect.height()) / 2.0;
    let [hg, hd, bd, bg] = coins.0.map(|c| c.clamp(0.0, demi));
    let mut points = Vec::with_capacity(8);
    let mut coin = |angle: f32, avant: Pos2, apres: Pos2, sommet: Pos2| {
        if angle > 0.0 {
            points.push(avant);
            points.push(apres);
        } else {
            points.push(sommet);
        }
    };
    let (g, d, h, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    coin(
        hg,
        egui::pos2(g, h + hg),
        egui::pos2(g + hg, h),
        rect.left_top(),
    );
    coin(
        hd,
        egui::pos2(d - hd, h),
        egui::pos2(d, h + hd),
        rect.right_top(),
    );
    coin(
        bd,
        egui::pos2(d, b - bd),
        egui::pos2(d - bd, b),
        rect.right_bottom(),
    );
    coin(
        bg,
        egui::pos2(g + bg, b),
        egui::pos2(g, b - bg),
        rect.left_bottom(),
    );
    points
}

/// Les six sommets d'un hexagone pointe en haut, inscrit dans `rect`.
pub fn hexagone(rect: Rect) -> Vec<Pos2> {
    let (g, d, h, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let (x, quart) = (rect.center().x, rect.height() / 4.0);
    vec![
        egui::pos2(x, h),
        egui::pos2(d, h + quart),
        egui::pos2(d, b - quart),
        egui::pos2(x, b),
        egui::pos2(g, b - quart),
        egui::pos2(g, h + quart),
    ]
}

/// Remplit un polygone convexe d'un dégradé : `debut` du côté opposé à
/// `direction`, `fin` du côté vers lequel elle pointe (par exemple
/// `Vec2::X` pour un dégradé de gauche à droite).
pub fn degrade(
    peintre: &egui::Painter,
    points: &[Pos2],
    debut: Color32,
    fin: Color32,
    direction: Vec2,
) {
    if points.len() < 3 {
        return;
    }
    let direction = direction.normalized();
    let projections: Vec<f32> = points.iter().map(|p| p.to_vec2().dot(direction)).collect();
    let min = projections.iter().copied().fold(f32::INFINITY, f32::min);
    let max = projections
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let etendue = (max - min).max(1.0);
    let mut maillage = epaint::Mesh::default();
    for (point, projection) in points.iter().zip(&projections) {
        let t = (projection - min) / etendue;
        maillage.colored_vertex(*point, melange(debut, fin, t));
    }
    // Un polygone convexe se découpe en éventail depuis son premier sommet.
    for i in 1..points.len() as u32 - 1 {
        maillage.add_triangle(0, i, i + 1);
    }
    peintre.add(Shape::mesh(maillage));
}

/// Un mélange de deux couleurs : `t` = 0 donne `a`, 1 donne `b`.
pub fn melange(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let canal = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        canal(a.r(), b.r()),
        canal(a.g(), b.g()),
        canal(a.b(), b.b()),
        canal(a.a(), b.a()),
    )
}

/// Une lueur floue autour de `rect` (une ombre colorée d'egui), plus ou
/// moins forte selon le réglage des effets. `force` : 1 pour une lueur normale.
pub fn halo(peintre: &egui::Painter, rect: Rect, couleur: Color32, force: f32) {
    let force = force * intensite();
    if force <= 0.0 {
        return;
    }
    let ombre = epaint::Shadow {
        offset: Vec2::ZERO,
        blur: 10.0 * force,
        spread: 1.0 * force,
        color: fondu(couleur, 0.55),
    };
    peintre.add(ombre.as_shape(rect, 0.0));
}

/// Une lueur autour d'un cadre à angles coupés : quelques contours de plus
/// en plus larges et transparents. Moins cher qu'un vrai flou.
pub fn lueur_contour(
    peintre: &egui::Painter,
    rect: Rect,
    coins: Coins,
    couleur: Color32,
    force: f32,
) {
    let force = force * intensite();
    if force <= 0.0 {
        return;
    }
    for (ecart, opacite) in [(1.5, 0.35), (3.5, 0.18), (6.0, 0.08)] {
        let ecart = ecart * force.sqrt();
        let points = contour(rect.expand(ecart), coins.elargis(ecart));
        let trait_ = Stroke::new(2.0_f32, fondu(couleur, opacite * force.min(1.0)));
        peintre.add(Shape::closed_line(points, trait_));
    }
}

/// Un cadre néon : fond uni (transparent : aucun), contour, et une lueur si `lueur` > 0.
pub fn cadre(
    peintre: &egui::Painter,
    rect: Rect,
    coins: Coins,
    fond: Color32,
    contour_: Stroke,
    lueur: f32,
) {
    if lueur > 0.0 {
        lueur_contour(peintre, rect, coins, contour_.color, lueur);
    }
    let points = contour(rect, coins);
    if fond.a() > 0 {
        peintre.add(Shape::convex_polygon(points.clone(), fond, Stroke::NONE));
    }
    if contour_.width > 0.0 {
        peintre.add(Shape::closed_line(points, contour_));
    }
}

/// Met en page un texte sur une ligne, lettres écartées de `espacement`.
pub fn mise_en_page(
    ctx: &egui::Context,
    texte: &str,
    police: egui::FontId,
    espacement: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        texte,
        0.0,
        egui::TextFormat {
            font_id: police,
            extra_letter_spacing: espacement,
            color: Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    ctx.fonts(|f| f.layout_job(job))
}

/// Écrit `texte` avec une lueur de `lueur` autour (pour les titres : le
/// texte est redessiné quelques fois, décalé et transparent). Les lettres
/// sont écartées de `espacement`, comme les titres Orbitron de la maquette.
#[allow(clippy::too_many_arguments)]
pub fn texte_lumineux(
    peintre: &egui::Painter,
    position: Pos2,
    ancre: egui::Align2,
    texte: &str,
    police: egui::FontId,
    espacement: f32,
    couleur: Color32,
    lueur: Color32,
) -> Rect {
    let galet = mise_en_page(peintre.ctx(), texte, police, espacement);
    let rect = ancre.anchor_size(position, galet.size());
    let force = intensite();
    if force > 0.0 {
        let halo = fondu(lueur, 0.22 * force.min(1.5));
        let r = 1.6 * force.sqrt();
        for (dx, dy) in [
            (r, 0.0),
            (-r, 0.0),
            (0.0, r),
            (0.0, -r),
            (r * 0.7, r * 0.7),
            (-r * 0.7, r * 0.7),
            (r * 0.7, -r * 0.7),
            (-r * 0.7, -r * 0.7),
        ] {
            peintre.galley_with_override_text_color(
                rect.min + egui::vec2(dx, dy),
                galet.clone(),
                halo,
            );
        }
    }
    peintre.galley_with_override_text_color(rect.min, galet, couleur);
    rect
}

/// Les icônes des boutons, dessinées au trait (comme celles de la maquette),
/// sans dépendre des émojis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icone {
    Precedent,
    Suivant,
    Lecture,
    Pause,
    Stop,
    Aleatoire,
    Boucle,
    Son,
    Muet,
    Options,
    Recherche,
    Plus,
    Croix,
    Dossier,
    Actualiser,
    Crayon,
    Corbeille,
}

/// Dessine une icône dans `rect` (un carré, de préférence), de `couleur`.
pub fn icone(peintre: &egui::Painter, icone: Icone, rect: Rect, couleur: Color32) {
    // Les icônes sont pensées sur une grille de 18 × 18, comme dans la maquette.
    let echelle = rect.width().min(rect.height()) / 18.0;
    let origine = rect.center() - egui::vec2(9.0, 9.0) * echelle;
    let p = |x: f32, y: f32| origine + egui::vec2(x, y) * echelle;
    let trait_ = Stroke::new(1.5 * echelle.max(0.8), couleur);
    let plein = |points: Vec<Pos2>| Shape::convex_polygon(points, couleur, Stroke::NONE);
    let ligne = |points: Vec<Pos2>| Shape::line(points, trait_);
    let formes = match icone {
        Icone::Precedent => vec![
            plein(vec![p(4.0, 3.0), p(5.6, 3.0), p(5.6, 15.0), p(4.0, 15.0)]),
            plein(vec![p(15.0, 3.0), p(15.0, 15.0), p(6.0, 9.0)]),
        ],
        Icone::Suivant => vec![
            plein(vec![
                p(12.4, 3.0),
                p(14.0, 3.0),
                p(14.0, 15.0),
                p(12.4, 15.0),
            ]),
            plein(vec![p(3.0, 3.0), p(12.0, 9.0), p(3.0, 15.0)]),
        ],
        Icone::Lecture => vec![plein(vec![p(5.0, 3.0), p(15.0, 9.0), p(5.0, 15.0)])],
        Icone::Pause => vec![
            plein(vec![p(5.0, 3.0), p(8.0, 3.0), p(8.0, 15.0), p(5.0, 15.0)]),
            plein(vec![
                p(10.0, 3.0),
                p(13.0, 3.0),
                p(13.0, 15.0),
                p(10.0, 15.0),
            ]),
        ],
        Icone::Stop => vec![plein(vec![
            p(4.0, 4.0),
            p(14.0, 4.0),
            p(14.0, 14.0),
            p(4.0, 14.0),
        ])],
        Icone::Aleatoire => vec![
            ligne(vec![p(1.0, 5.0), p(5.0, 5.0), p(12.0, 13.0), p(16.0, 13.0)]),
            ligne(vec![p(1.0, 13.0), p(5.0, 13.0), p(7.0, 10.7)]),
            ligne(vec![p(10.0, 7.3), p(12.0, 5.0), p(16.0, 5.0)]),
            ligne(vec![p(14.0, 3.0), p(16.0, 5.0), p(14.0, 7.0)]),
            ligne(vec![p(14.0, 11.0), p(16.0, 13.0), p(14.0, 15.0)]),
        ],
        Icone::Boucle => vec![
            ligne(vec![p(2.0, 8.0), p(2.0, 5.0), p(14.0, 5.0)]),
            ligne(vec![p(12.0, 3.0), p(14.0, 5.0), p(12.0, 7.0)]),
            ligne(vec![p(16.0, 10.0), p(16.0, 13.0), p(4.0, 13.0)]),
            ligne(vec![p(6.0, 15.0), p(4.0, 13.0), p(6.0, 11.0)]),
        ],
        Icone::Son | Icone::Muet => {
            let mut formes = vec![plein(vec![
                p(2.0, 7.0),
                p(5.0, 7.0),
                p(9.0, 4.0),
                p(9.0, 14.0),
                p(5.0, 11.0),
                p(2.0, 11.0),
            ])];
            if icone == Icone::Son {
                formes.push(Shape::line(
                    arc(p(9.5, 9.0), 3.5 * echelle, -0.9, 0.9),
                    trait_,
                ));
                formes.push(Shape::line(
                    arc(p(9.5, 9.0), 6.0 * echelle, -1.0, 1.0),
                    trait_,
                ));
            } else {
                formes.push(ligne(vec![p(11.5, 6.5), p(16.5, 11.5)]));
                formes.push(ligne(vec![p(16.5, 6.5), p(11.5, 11.5)]));
            }
            formes
        }
        Icone::Options => {
            let centre = p(9.0, 9.0);
            let mut formes = vec![Shape::circle_stroke(centre, 2.8 * echelle, trait_)];
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::FRAC_PI_4;
                let dir = egui::vec2(angle.cos(), angle.sin());
                formes.push(ligne(vec![
                    centre + dir * 5.0 * echelle,
                    centre + dir * 7.5 * echelle,
                ]));
            }
            formes
        }
        Icone::Recherche => vec![
            Shape::circle_stroke(p(7.5, 7.5), 5.0 * echelle, trait_),
            ligne(vec![p(11.3, 11.3), p(15.5, 15.5)]),
        ],
        Icone::Plus => vec![
            ligne(vec![p(9.0, 3.0), p(9.0, 15.0)]),
            ligne(vec![p(3.0, 9.0), p(15.0, 9.0)]),
        ],
        Icone::Croix => vec![
            ligne(vec![p(4.0, 4.0), p(14.0, 14.0)]),
            ligne(vec![p(14.0, 4.0), p(4.0, 14.0)]),
        ],
        Icone::Dossier => vec![Shape::closed_line(
            vec![
                p(2.0, 4.0),
                p(7.0, 4.0),
                p(8.5, 6.0),
                p(16.0, 6.0),
                p(16.0, 14.5),
                p(2.0, 14.5),
            ],
            trait_,
        )],
        Icone::Actualiser => vec![
            Shape::line(arc(p(9.0, 9.0), 5.5 * echelle, -2.6, 2.0), trait_),
            ligne(vec![p(10.0, 1.5), p(13.5, 3.8), p(10.5, 6.5)]),
        ],
        Icone::Crayon => vec![
            Shape::closed_line(
                vec![
                    p(12.0, 3.0),
                    p(15.0, 6.0),
                    p(7.0, 14.0),
                    p(3.5, 14.5),
                    p(4.0, 11.0),
                ],
                trait_,
            ),
            ligne(vec![p(10.5, 4.5), p(13.5, 7.5)]),
        ],
        Icone::Corbeille => vec![
            ligne(vec![p(3.0, 5.0), p(15.0, 5.0)]),
            ligne(vec![p(7.0, 5.0), p(7.5, 3.0), p(10.5, 3.0), p(11.0, 5.0)]),
            ligne(vec![p(4.5, 5.0), p(5.5, 15.0), p(12.5, 15.0), p(13.5, 5.0)]),
            ligne(vec![p(7.5, 8.0), p(7.5, 12.5)]),
            ligne(vec![p(10.5, 8.0), p(10.5, 12.5)]),
        ],
    };
    peintre.extend(formes);
}

/// Les points d'un arc de cercle, d'un angle à l'autre (en radians, 0 = à droite).
fn arc(centre: Pos2, rayon: f32, de: f32, a: f32) -> Vec<Pos2> {
    (0..=12)
        .map(|i| {
            let angle = de + (a - de) * i as f32 / 12.0;
            centre + egui::vec2(angle.cos(), angle.sin()) * rayon
        })
        .collect()
}

/// L'allure d'un bouton néon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Allure {
    /// Contour de la couleur, fond de panneau (les boutons de lecture).
    Contour,
    /// Fond teinté de la couleur et lueur : un bouton « allumé » (Aléa actif,
    /// onglet choisi, OPTIONS).
    Allume,
    /// Contour pâle et texte discret : un bouton éteint ou secondaire.
    Eteint,
    /// Hexagone en dégradé cyan → magenta, avec lueur (le gros bouton Lecture).
    Hexagone,
}

/// Un bouton néon : une icône et/ou un texte (en majuscules), dans un cadre
/// à angles coupés. S'utilise comme un bouton egui : `ui.add(Bouton::…)`.
pub struct Bouton {
    icone: Option<Icone>,
    texte: Option<String>,
    couleur: Color32,
    allure: Allure,
    coins: Coins,
    taille: Option<Vec2>,
    taille_texte: f32,
}

impl Bouton {
    /// Un bouton carré avec une icône seule.
    pub fn icone(icone: Icone) -> Self {
        Bouton {
            icone: Some(icone),
            texte: None,
            couleur: CYAN,
            allure: Allure::Contour,
            coins: Coins::AUCUN,
            taille: None,
            taille_texte: 14.5,
        }
    }

    /// Un bouton avec un texte (mis en majuscules).
    pub fn texte(texte: impl Into<String>) -> Self {
        Bouton {
            texte: Some(texte.into().to_uppercase()),
            icone: None,
            ..Bouton::icone(Icone::Plus)
        }
    }

    /// Ajoute une icône devant le texte.
    pub fn avec_icone(mut self, icone: Icone) -> Self {
        self.icone = Some(icone);
        self
    }

    pub fn couleur(mut self, couleur: Color32) -> Self {
        self.couleur = couleur;
        self
    }

    pub fn allure(mut self, allure: Allure) -> Self {
        self.allure = allure;
        self
    }

    /// Allumé ou éteint, selon `allume` (pour un interrupteur comme Aléa).
    pub fn allume(self, allume: bool) -> Self {
        self.allure(if allume {
            Allure::Allume
        } else {
            Allure::Eteint
        })
    }

    pub fn coins(mut self, coins: Coins) -> Self {
        self.coins = coins;
        self
    }

    /// La taille exacte du bouton (sinon, elle suit son contenu).
    pub fn taille(mut self, taille: Vec2) -> Self {
        self.taille = Some(taille);
        self
    }

    /// Un bouton plus petit (les boutons des en-têtes, « + NOUVELLE »).
    pub fn petit(mut self) -> Self {
        self.taille_texte = 13.0;
        self
    }

    /// La mise en page du texte, s'il y en a un.
    fn mise_en_page(&self, ui: &egui::Ui) -> Option<std::sync::Arc<egui::Galley>> {
        let texte = self.texte.as_ref()?;
        let police = egui::FontId::proportional(self.taille_texte);
        Some(mise_en_page(ui.ctx(), texte, police, 1.0))
    }
}

impl egui::Widget for Bouton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let galet = self.mise_en_page(ui);
        let cote_icone = if galet.is_some() { 14.0 } else { 18.0 };
        let taille = self.taille.unwrap_or_else(|| {
            let largeur_texte = galet.as_ref().map_or(0.0, |g| g.size().x);
            let ecart = if galet.is_some() && self.icone.is_some() {
                6.0
            } else {
                0.0
            };
            let largeur_icone = if self.icone.is_some() {
                cote_icone
            } else {
                0.0
            };
            let hauteur = if galet.is_some() { 28.0 } else { 34.0 };
            let largeur = largeur_icone + ecart + largeur_texte + 2.0 * 11.0;
            egui::vec2(largeur.max(hauteur), hauteur)
        });
        let (rect, reponse) = ui.allocate_exact_size(taille, egui::Sense::click());
        if !ui.is_rect_visible(rect) {
            return reponse;
        }
        let actif = ui.is_enabled();
        let survol = actif && reponse.hovered();
        let appui = actif && reponse.is_pointer_button_down_on();
        let attenuation = if actif { 1.0 } else { 0.35 };
        let peintre = ui.painter();
        let c = fondu(self.couleur, attenuation);

        let encre = match self.allure {
            Allure::Hexagone => {
                let points = hexagone(rect);
                // La lueur : un halo cyan derrière l'hexagone.
                halo(
                    peintre,
                    rect.shrink(rect.width() * 0.18),
                    c,
                    if survol { 2.0 } else { 0.9 + 0.4 * pulsation() },
                );
                let (debut, fin) = if appui {
                    (super::MAGENTA, CYAN)
                } else {
                    (CYAN, super::MAGENTA)
                };
                degrade(
                    peintre,
                    &points,
                    fondu(debut, attenuation),
                    fondu(fin, attenuation),
                    egui::vec2(1.0, 1.0),
                );
                if survol {
                    peintre.add(Shape::closed_line(
                        points,
                        Stroke::new(1.5_f32, fondu(Color32::WHITE, 0.7)),
                    ));
                }
                FOND
            }
            Allure::Contour => {
                let fond = if appui {
                    attenue(self.couleur, 60)
                } else if survol {
                    attenue(self.couleur, 28)
                } else {
                    PANNEAU
                };
                let bord = if survol {
                    c
                } else {
                    fondu(self.couleur, 0.55 * attenuation)
                };
                cadre(
                    peintre,
                    rect,
                    self.coins,
                    fondu(fond, attenuation),
                    Stroke::new(1.0_f32, bord),
                    if survol { 0.8 } else { 0.0 },
                );
                c
            }
            Allure::Allume => {
                let fond = attenue(self.couleur, if survol || appui { 70 } else { 46 });
                cadre(
                    peintre,
                    rect,
                    self.coins,
                    fondu(fond, attenuation),
                    Stroke::new(1.0_f32, c),
                    if survol { 1.2 } else { 0.8 },
                );
                c
            }
            Allure::Eteint => {
                let bord = fondu(self.couleur, if survol { 0.8 } else { 0.4 } * attenuation);
                let fond = if survol {
                    attenue(self.couleur, 20)
                } else {
                    Color32::TRANSPARENT
                };
                cadre(
                    peintre,
                    rect,
                    self.coins,
                    fond,
                    Stroke::new(1.0_f32, bord),
                    0.0,
                );
                if survol {
                    c
                } else {
                    fondu(TEXTE_DISCRET, attenuation)
                }
            }
        };

        // L'icône puis le texte, centrés ensemble.
        let largeur_texte = galet.as_ref().map_or(0.0, |g| g.size().x);
        let ecart = if galet.is_some() && self.icone.is_some() {
            6.0
        } else {
            0.0
        };
        let largeur_icone = if self.icone.is_some() {
            cote_icone
        } else {
            0.0
        };
        let cote = if self.allure == Allure::Hexagone {
            rect.height() * 0.38
        } else {
            largeur_icone
        };
        let total = if galet.is_some() {
            largeur_icone + ecart + largeur_texte
        } else {
            cote
        };
        let mut x = rect.center().x - total / 2.0;
        if let Some(i) = self.icone {
            let zone = Rect::from_min_size(
                egui::pos2(x, rect.center().y - cote / 2.0),
                Vec2::splat(cote),
            );
            // Le triangle « Lecture » paraît mieux centré un peu à droite.
            let zone = if i == Icone::Lecture && galet.is_none() {
                zone.translate(egui::vec2(cote * 0.08, 0.0))
            } else {
                zone
            };
            icone(peintre, i, zone, encre);
            x += cote + ecart;
        }
        if let Some(galet) = galet {
            let y = rect.center().y - galet.size().y / 2.0;
            peintre.galley_with_override_text_color(egui::pos2(x, y), galet, encre);
        }
        reponse
    }
}

/// L'en-tête d'une section du panneau (« BIBLIOTHÈQUE », « PLAYLISTS ») :
/// une petite barre lumineuse, le titre en Orbitron de la couleur de la
/// section, puis `droite` (nombre de titres, boutons), calé à droite.
pub fn entete_section<R>(
    ui: &mut egui::Ui,
    titre: &str,
    couleur: Color32,
    droite: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        let (barre, _) = ui.allocate_exact_size(egui::vec2(4.0, 18.0), egui::Sense::hover());
        halo(ui.painter(), barre, couleur, 0.8);
        ui.painter().rect_filled(barre, 0.0, couleur);
        ui.add_space(2.0);
        titre_neon(ui, titre, couleur);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), droite)
            .inner
    })
    .inner
}

/// Un titre de section en Orbitron lumineux, à sa place dans une rangée
/// (« LISTE DE LECTURE »).
pub fn titre_neon(ui: &mut egui::Ui, titre: &str, couleur: Color32) -> egui::Response {
    let police = crate::police::titres(ui.ctx(), 13.0);
    let texte = titre.to_uppercase();
    let taille = mise_en_page(ui.ctx(), &texte, police.clone(), 2.5).size();
    let (rect, reponse) = ui.allocate_exact_size(taille, egui::Sense::hover());
    texte_lumineux(
        ui.painter(),
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        &texte,
        police,
        2.5,
        couleur,
        couleur,
    );
    reponse
}

/// Un trait horizontal qui s'éteint sur les bords (séparateur néon).
pub fn separateur(ui: &mut egui::Ui, couleur: Color32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 9.0), egui::Sense::hover());
    let y = rect.center().y;
    let points = [
        egui::pos2(rect.left(), y - 0.5),
        egui::pos2(rect.center().x, y - 0.5),
        egui::pos2(rect.right(), y - 0.5),
        egui::pos2(rect.right(), y + 0.5),
        egui::pos2(rect.center().x, y + 0.5),
        egui::pos2(rect.left(), y + 0.5),
    ];
    let mut maillage = epaint::Mesh::default();
    for (n, p) in points.iter().enumerate() {
        let c = if n == 1 || n == 4 {
            couleur
        } else {
            Color32::TRANSPARENT
        };
        maillage.colored_vertex(*p, c);
    }
    maillage.add_triangle(0, 1, 4);
    maillage.add_triangle(0, 4, 5);
    maillage.add_triangle(1, 2, 3);
    maillage.add_triangle(1, 3, 4);
    ui.painter().add(Shape::mesh(maillage));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carre() -> Rect {
        Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 40.0))
    }

    #[test]
    fn un_cadre_sans_angle_coupe_est_un_rectangle() {
        assert_eq!(
            contour(carre(), Coins::AUCUN),
            vec![
                egui::pos2(0.0, 0.0),
                egui::pos2(100.0, 0.0),
                egui::pos2(100.0, 40.0),
                egui::pos2(0.0, 40.0)
            ]
        );
    }

    #[test]
    fn chaque_angle_coupe_ajoute_un_sommet() {
        let points = contour(carre(), Coins::diagonale(10.0));
        assert_eq!(points.len(), 6);
        assert_eq!(points[0], egui::pos2(0.0, 10.0));
        assert_eq!(points[1], egui::pos2(10.0, 0.0));
        assert_eq!(contour(carre(), Coins([8.0; 4])).len(), 8);
        // Un angle trop grand est ramené à la moitié du petit côté.
        let points = contour(carre(), Coins::haut_gauche(500.0));
        assert_eq!(points[0], egui::pos2(0.0, 20.0));
    }

    #[test]
    fn l_hexagone_touche_les_bords_de_son_cadre() {
        let points = hexagone(Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(64.0, 64.0),
        ));
        assert_eq!(points.len(), 6);
        assert_eq!(points[0], egui::pos2(32.0, 0.0));
        assert_eq!(points[3], egui::pos2(32.0, 64.0));
        assert!(points.iter().all(|p| (0.0..=64.0).contains(&p.x)));
    }

    #[test]
    fn le_melange_va_d_une_couleur_a_l_autre() {
        assert_eq!(melange(CYAN, super::super::MAGENTA, 0.0), CYAN);
        assert_eq!(
            melange(CYAN, super::super::MAGENTA, 1.0),
            super::super::MAGENTA
        );
        let milieu = melange(Color32::BLACK, Color32::WHITE, 0.5);
        assert!((127..=128).contains(&milieu.r()));
    }

    #[test]
    fn l_intensite_reste_entre_0_et_2() {
        fixer_intensite(5.0);
        assert_eq!(intensite(), 2.0);
        fixer_intensite(-1.0);
        assert_eq!(intensite(), 0.0);
        fixer_intensite(1.0);
        assert_eq!(intensite(), 1.0);
    }

    /// Dessine une image avec un bouton néon, en cliquant à `clic` s'il y en a
    /// un. Renvoie la place du bouton et s'il a été cliqué.
    fn image_avec_bouton(ctx: &egui::Context, clic: Option<Pos2>) -> (Rect, bool) {
        let mut entree = egui::RawInput::default();
        if let Some(pos) = clic {
            let appui = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            entree.events = vec![egui::Event::PointerMoved(pos), appui(true), appui(false)];
        }
        let mut resultat = (Rect::NOTHING, false);
        let _ = ctx.run(entree, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let reponse = ui.add(Bouton::texte("Options").avec_icone(Icone::Options));
                resultat = (reponse.rect, reponse.clicked());
            });
        });
        resultat
    }

    #[test]
    fn un_bouton_neon_se_clique_comme_un_bouton() {
        let ctx = egui::Context::default();
        crate::police::installer(&ctx);
        let (rect, clique) = image_avec_bouton(&ctx, None);
        assert!(!clique);
        assert!(rect.width() > 60.0, "le texte tient dans le bouton");
        let (_, clique) = image_avec_bouton(&ctx, Some(rect.center()));
        assert!(clique);
    }
}
