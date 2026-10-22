//! La vraie courbure de l'écran cathodique : toute l'image est bombée,
//! comme sur le verre d'une vieille télé.
//!
//! egui ne sait pas déformer l'image une fois dessinée. On déforme donc
//! ce qu'il va dessiner : à la fin de chaque image, toutes les formes
//! (cadres, textes, pochette…) sont découpées en triangles dont on
//! déplace les coins vers le milieu de l'écran. Les clics de la souris
//! suivent le chemin inverse, pour tomber pile sur ce qu'on voit.

use std::collections::HashMap;

use eframe::egui::{
    self,
    epaint::{ClippedShape, Color32, Mesh, Tessellator, Vertex},
    layers::ShapeIdx,
    LayerId, Pos2, Rect, Shape,
};

/// La force du bombé : aux coins, l'image se resserre d'environ 3 %.
const FORCE: f32 = 0.07;
/// La taille maximale d'un triangle d'image (en points), pour que
/// l'intérieur des grandes images (la pochette) se bombe aussi.
const PAS: f32 = 128.0;
/// L'écart toléré (en points) entre un côté découpé et la vraie courbe.
const ECART_MAX: f32 = 0.5;
/// Un côté plus court que ça (en points) s'écarte toujours moins de
/// `ECART_MAX` de la vraie courbe : pas besoin de le vérifier.
const PETIT: f32 = 48.0;

/// Où un point de l'image arrive sur l'écran bombé : rien ne bouge au
/// milieu ni au milieu des bords, les coins se rapprochent du centre.
/// (Appelé des dizaines de milliers de fois par image : un calcul direct.)
pub fn bomber(ecran: Rect, p: Pos2) -> Pos2 {
    let (centre, demi) = (ecran.center(), ecran.size() / 2.0);
    let s = (p - centre) / demi;
    let d = egui::vec2(
        s.x / (1.0 + FORCE * s.y * s.y),
        s.y / (1.0 + FORCE * s.x * s.x),
    );
    centre + d * demi
}

/// L'inverse : d'où vient, dans l'image, un point de l'écran bombé
/// (pour savoir sur quoi la souris clique).
pub fn redresser(ecran: Rect, p: Pos2) -> Pos2 {
    let (centre, demi) = (ecran.center(), ecran.size() / 2.0);
    let d = (p - centre) / demi;
    // L'inverse de `bomber`, trouvé par petites corrections successives
    // (le bombé est léger : huit suffisent largement).
    let mut s = d;
    for _ in 0..8 {
        s = egui::vec2(
            d.x * (1.0 + FORCE * s.y * s.y),
            d.y * (1.0 + FORCE * s.x * s.x),
        );
    }
    centre + s * demi
}

/// Bombe tout ce qui a été dessiné à cette image, sur les calques
/// `calques` (ceux des fenêtres et menus sont ajoutés d'office).
/// À appeler une seule fois, tout à la fin de l'image.
pub fn courber(ctx: &egui::Context, calques: &[LayerId]) {
    let ecran = ctx.screen_rect();
    let options = ctx.tessellation_options(|o| *o);
    let (taille_police, disques) = ctx.fonts(|f| {
        let atlas = f.texture_atlas();
        let atlas = atlas.lock();
        (atlas.size(), atlas.prepared_discs())
    });
    let mut decoupe = Tessellator::new(ctx.pixels_per_point(), options, taille_police, disques);

    let mut tous: Vec<LayerId> = ctx.memory(|m| m.layer_ids().collect());
    for calque in calques {
        if !tous.contains(calque) {
            tous.push(*calque);
        }
    }
    ctx.graphics_mut(|g| {
        for calque in tous {
            let Some(liste) = g.get_mut(calque) else {
                continue;
            };
            let formes: Vec<ClippedShape> = liste.all_entries().cloned().collect();
            for (i, forme) in formes.into_iter().enumerate() {
                let cadre = bomber_cadre(ecran, forme.clip_rect);
                let courbee = bomber_forme(&mut decoupe, ecran, forme.shape);
                liste.set(ShapeIdx(i), cadre, courbee);
            }
        }
    });
}

/// Une forme d'egui, découpée en petits triangles puis bombée.
fn bomber_forme(decoupe: &mut Tessellator, ecran: Rect, forme: Shape) -> Shape {
    match forme {
        Shape::Noop => Shape::Noop,
        // Dessin libre du système : on ne sait pas le déformer.
        Shape::Callback(_) => forme,
        Shape::Vec(formes) => Shape::Vec(
            formes
                .into_iter()
                .map(|f| bomber_forme(decoupe, ecran, f))
                .collect(),
        ),
        forme => {
            let mut maillage = Mesh::default();
            decoupe.tessellate_shape(forme, &mut maillage);
            Shape::Mesh(bomber_maillage(&maillage, ecran))
        }
    }
}

/// Le rectangle de découpe d'une forme, une fois bombé : le plus petit
/// rectangle qui contient ses bords bombés.
fn bomber_cadre(ecran: Rect, cadre: Rect) -> Rect {
    if !cadre.is_finite() || !cadre.is_positive() {
        return cadre;
    }
    let mut resultat = Rect::NOTHING;
    for i in 0..=8 {
        let t = i as f32 / 8.0;
        for p in [
            cadre.lerp_inside(egui::vec2(t, 0.0)),
            cadre.lerp_inside(egui::vec2(t, 1.0)),
            cadre.lerp_inside(egui::vec2(0.0, t)),
            cadre.lerp_inside(egui::vec2(1.0, t)),
        ] {
            resultat.extend_with(bomber(ecran, p));
        }
    }
    resultat.expand(1.0)
}

/// Le même maillage, bombé. Pour que les bords droits deviennent de
/// belles courbes, on coupe en deux les côtés qui, une fois bombés,
/// s'écarteraient de plus de `ECART_MAX` de la vraie courbe (et, pour les
/// images, ceux plus longs que `PAS`, pour que leur intérieur suive aussi).
/// Un côté est coupé ou non selon ses seuls deux bouts, et son milieu est
/// partagé : deux triangles voisins coupent donc leur côté commun au même
/// point, et rien ne se décolle.
fn bomber_maillage(maillage: &Mesh, ecran: Rect) -> Mesh {
    let mut sortie = Mesh::with_texture(maillage.texture_id);
    sortie.vertices = maillage.vertices.clone();
    let mut bombes: Vec<Pos2> = sortie
        .vertices
        .iter()
        .map(|v| bomber(ecran, v.pos))
        .collect();
    let mut pile: Vec<[u32; 3]> = maillage
        .indices
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]])
        .collect();
    let mut milieux: HashMap<(Cle, Cle), u32> = HashMap::new();
    // Une vraie image (la pochette), pas du texte ni une forme unie.
    let image = maillage.texture_id != egui::TextureId::default();
    while let Some(t) = pile.pop() {
        let [p0, p1, p2] = t.map(|i| sortie.vertices[i as usize].pos);
        let long_max = p0
            .distance_sq(p1)
            .max(p1.distance_sq(p2))
            .max(p2.distance_sq(p0));
        if !image && long_max < PETIT * PETIT {
            // Petit triangle (une lettre…) : il se courbe à peine, inutile
            // de vérifier ses côtés.
            sortie.indices.extend_from_slice(&t);
            continue;
        }
        let double_aire = ((p1 - p0).x * (p2 - p0).y - (p1 - p0).y * (p2 - p0).x).abs();
        if double_aire <= 1e-6 * long_max {
            // Triangle plat, invisible : inutile de le découper.
            sortie.indices.extend_from_slice(&t);
            continue;
        }
        // Le plus long des côtés à couper : de t[k] à t[(k + 1) % 3].
        let mut a_couper: Option<(usize, f32, Pos2)> = None;
        for k in 0..3 {
            let (i, j) = (t[k] as usize, t[(k + 1) % 3] as usize);
            let (p, q) = (sortie.vertices[i].pos, sortie.vertices[j].pos);
            let long2 = p.distance_sq(q);
            if long2 < 4.0 || a_couper.is_some_and(|(_, l, _)| l >= long2) {
                continue;
            }
            let milieu = bomber(ecran, p.lerp(q, 0.5));
            let ecart = milieu.distance(bombes[i].lerp(bombes[j], 0.5));
            if (image && long2 > PAS * PAS) || ecart > ECART_MAX {
                a_couper = Some((k, long2, milieu));
            }
        }
        let Some((k, _, bombe)) = a_couper else {
            sortie.indices.extend_from_slice(&t);
            continue;
        };
        let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
        // Le voisin qui partage ce côté réutilise le même milieu.
        let cle = |i: u32| {
            let p = sortie.vertices[i as usize].pos;
            (p.x.to_bits(), p.y.to_bits())
        };
        let (ca, cb) = (cle(a), cle(b));
        let milieu = *milieux.entry((ca.min(cb), ca.max(cb))).or_insert_with(|| {
            let (va, vb) = (sortie.vertices[a as usize], sortie.vertices[b as usize]);
            sortie.vertices.push(Vertex {
                pos: va.pos.lerp(vb.pos, 0.5),
                uv: va.uv.lerp(vb.uv, 0.5),
                color: moyenne(va.color, vb.color),
            });
            bombes.push(bombe);
            sortie.vertices.len() as u32 - 1
        });
        pile.push([a, milieu, c]);
        pile.push([milieu, b, c]);
    }
    for (v, p) in sortie.vertices.iter_mut().zip(bombes) {
        v.pos = p;
    }
    sortie
}

/// Un point repéré par ses coordonnées exactes.
type Cle = (u32, u32);

/// La couleur à mi-chemin entre deux couleurs.
fn moyenne(a: Color32, b: Color32) -> Color32 {
    let m = |x: u8, y: u8| (x as u16 + y as u16).div_ceil(2) as u8;
    Color32::from_rgba_premultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}

/// Les clics et mouvements de la souris, ramenés de l'écran bombé vers
/// l'image, pour qu'ils tombent pile sur ce qu'on voit.
pub fn redresser_entree(ecran: Rect, entree: &mut egui::RawInput) {
    for evenement in &mut entree.events {
        match evenement {
            egui::Event::PointerMoved(pos) | egui::Event::PointerButton { pos, .. } => {
                *pos = redresser(ecran, *pos);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ecran() -> Rect {
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 800.0))
    }

    #[test]
    fn le_milieu_ne_bouge_pas_et_les_coins_rentrent() {
        let e = ecran();
        assert_eq!(bomber(e, e.center()), e.center());
        // Le milieu des bords reste au bord.
        assert!((bomber(e, egui::pos2(640.0, 0.0)).y).abs() < 1e-3);
        assert!((bomber(e, egui::pos2(0.0, 400.0)).x).abs() < 1e-3);
        // Les coins se rapprochent du centre, d'environ 3 %.
        let coin = bomber(e, e.min);
        assert!(coin.x > 10.0 && coin.x < 50.0, "{coin:?}");
        assert!(coin.y > 10.0 && coin.y < 30.0, "{coin:?}");
    }

    #[test]
    fn un_clic_tombe_pile_sur_ce_qu_on_voit() {
        let e = ecran();
        for p in [
            e.min,
            e.max,
            egui::pos2(100.0, 700.0),
            egui::pos2(900.0, 50.0),
            e.center(),
        ] {
            let retour = redresser(e, bomber(e, p));
            assert!(retour.distance(p) < 0.01, "{p:?} → {retour:?}");
        }
    }

    #[test]
    fn les_bords_droits_deviennent_des_courbes() {
        let e = ecran();
        let mut m = Mesh::default();
        m.add_colored_rect(e, Color32::RED);
        let fin = bomber_maillage(&m, e);
        assert!(
            fin.vertices.len() > 40,
            "bords découpés : {}",
            fin.vertices.len()
        );
        for t in fin.indices.chunks_exact(3) {
            let p = |i: u32| redresser(e, fin.vertices[i as usize].pos);
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let long_max = a.distance_sq(b).max(b.distance_sq(c)).max(c.distance_sq(a));
            if ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() <= 1e-5 * long_max {
                continue; // Triangle plat, invisible.
            }
            for k in 0..3 {
                let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
                // Chaque côté suit la vraie courbe à moins d'un demi-point.
                let (pa, pb) = (
                    redresser(e, fin.vertices[a].pos),
                    redresser(e, fin.vertices[b].pos),
                );
                let vrai = bomber(e, pa.lerp(pb, 0.5));
                let trace = fin.vertices[a].pos.lerp(fin.vertices[b].pos, 0.5);
                assert!(
                    vrai.distance(trace) <= ECART_MAX + 0.01,
                    "{pa:?} {pb:?} {:?} écart {}",
                    p(t[(k + 2) % 3]),
                    vrai.distance(trace)
                );
            }
        }
        // La surface redressée est la même : rien de perdu, rien en double.
        let aire: f32 = fin
            .indices
            .chunks_exact(3)
            .map(|t| {
                let p = |i: u32| redresser(e, fin.vertices[i as usize].pos);
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() / 2.0
            })
            .sum();
        assert!((aire - 1280.0 * 800.0).abs() < 2.0, "{aire}");
        assert!(fin.vertices.iter().all(|v| v.color == Color32::RED));
    }

    #[test]
    fn l_interieur_des_images_suit_aussi() {
        let e = ecran();
        let mut m = Mesh::with_texture(egui::TextureId::User(1));
        m.add_rect_with_uv(
            e,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let fin = bomber_maillage(&m, e);
        for t in fin.indices.chunks_exact(3) {
            for k in 0..3 {
                let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
                let (pa, pb) = (
                    redresser(e, fin.vertices[a].pos),
                    redresser(e, fin.vertices[b].pos),
                );
                assert!(pa.distance(pb) <= PAS + 0.01);
            }
        }
    }

    #[test]
    fn les_petits_cotes_suivent_toujours_la_courbe() {
        // Le pire endroit : un coin de l'écran.
        let e = ecran();
        for (p, q) in [
            (egui::pos2(0.0, 0.0), egui::pos2(PETIT, 0.0)),
            (egui::pos2(0.0, 0.0), egui::pos2(0.0, PETIT)),
            (
                egui::pos2(0.0, 0.0),
                egui::pos2(PETIT, PETIT) / 2.0_f32.sqrt(),
            ),
        ] {
            let vrai = bomber(e, p.lerp(q, 0.5));
            let trace = bomber(e, p).lerp(bomber(e, q), 0.5);
            assert!(vrai.distance(trace) < ECART_MAX, "{}", vrai.distance(trace));
        }
    }

    #[test]
    fn au_milieu_on_ne_decoupe_que_les_grands_cotes() {
        let e = ecran();
        let mut m = Mesh::default();
        m.add_colored_rect(
            Rect::from_center_size(e.center(), egui::vec2(60.0, 40.0)),
            Color32::RED,
        );
        assert_eq!(
            bomber_maillage(&m, e).vertices.len(),
            4,
            "petit et au centre : intact"
        );
    }

    #[test]
    fn un_rectangle_bombe_reste_dans_son_cadre_bombe() {
        let e = ecran();
        let cadre = Rect::from_min_max(egui::pos2(20.0, 30.0), egui::pos2(300.0, 200.0));
        let bombe = bomber_cadre(e, cadre);
        for i in 0..=20 {
            for j in 0..=20 {
                let p = cadre.lerp_inside(egui::vec2(i as f32 / 20.0, j as f32 / 20.0));
                assert!(bombe.contains(bomber(e, p)));
            }
        }
        assert_eq!(bomber_cadre(e, Rect::EVERYTHING), Rect::EVERYTHING);
    }

    #[test]
    fn hors_du_bombe_tout_est_redresse() {
        let e = ecran();
        let mut entree = egui::RawInput::default();
        let vu = bomber(e, egui::pos2(50.0, 60.0));
        entree.events.push(egui::Event::PointerMoved(vu));
        redresser_entree(e, &mut entree);
        let egui::Event::PointerMoved(p) = entree.events[0] else {
            panic!()
        };
        assert!(p.distance(egui::pos2(50.0, 60.0)) < 0.01);
    }

    #[test]
    fn la_moyenne_des_couleurs() {
        assert_eq!(
            moyenne(
                Color32::from_rgba_premultiplied(0, 100, 200, 255),
                Color32::TRANSPARENT
            ),
            Color32::from_rgba_premultiplied(0, 50, 100, 128)
        );
    }
}
