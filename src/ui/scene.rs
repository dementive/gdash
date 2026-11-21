//! Reads a hero image for places where something would naturally move or give off light, so
//! effects come out of the picture itself instead of being laid over it:
//!
//! - **fire** (hot, saturated, brighter than its surroundings): flickers and sheds embers;
//! - **lights** (small bright spots) and **stars** (specks in a dark sky): twinkle, and now
//!   and then flash a glint;
//! - **water** (from segmentation): shimmers and catches glints;
//! - **night sky** (segmentation + darkness): stars twinkle;
//! - **foliage** (segmentation): drops the odd leaf.
//!
//! Colour cues work on any image. Segmentation (`segment.png` from the enhancer: R sky,
//! G water, B foliage, A people) adds the rest when it's available.

use bevy::math::{Vec2, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmitKind {
    Ember,
    Glint,
    Star,
    Leaf,
    Mote,
}

/// A place in the image that gives off particles.
#[derive(Clone, Debug)]
pub struct Emitter {
    pub kind: EmitKind,
    /// Image UV.
    pub uv: Vec2,
    /// Card depth there (so particles ride the same parallax layer).
    pub depth: f32,
    pub strength: f32,
    /// Linear colour of the image there, normalised to max 1.
    pub color: Vec3,
}

pub struct Analysis {
    /// RGBA8 at map resolution: R fire, G twinkle (lights, stars), B water, A sky. Drives the
    /// in-image animation.
    pub fxmap: Vec<u8>,
    pub emitters: Vec<Emitter>,
}

/// People/animals per map pixel (0..1), from segmentation, if any.
pub fn subjects(seg: Option<&[u8]>) -> Option<Vec<f32>> {
    seg.map(|s| s.chunks_exact(4).map(|p| p[3] as f32 / 255.0).collect())
}

/// Loads `segment.png` resampled to the depth map's size.
pub fn load_segment(id: &str, w: u32, h: u32) -> Option<Vec<u8>> {
    let img = image::open(crate::art::game_dir(id).join("segment.png")).ok()?.to_rgba8();
    Some(image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle).into_raw())
}

/// `px` is the hero (RGBA8, `iw`x`ih`); `cards` the layered depth map (`w`x`h`).
pub fn analyse(iw: u32, ih: u32, px: &[u8], w: u32, h: u32, cards: &[u8], seg: Option<&[u8]>) -> Analysis {
    let (w, h, iw, ih) = (w as usize, h as usize, iw as usize, ih as usize);
    let n = w * h;
    // Per map pixel: average colour and the brightest source pixel (small lights matter).
    let mut avg = vec![[0f32; 3]; n];
    let mut peak = vec![0f32; n];
    for y in 0..h {
        let (y0, y1) = (y * ih / h, ((y + 1) * ih / h).max(y * ih / h + 1).min(ih));
        for x in 0..w {
            let (x0, x1) = (x * iw / w, ((x + 1) * iw / w).max(x * iw / w + 1).min(iw));
            let (mut s, mut m, mut c) = ([0f32; 3], 0f32, 0f32);
            for sy in (y0..y1).step_by(((y1 - y0) / 3).max(1)) {
                for sx in (x0..x1).step_by(((x1 - x0) / 3).max(1)) {
                    let o = (sy * iw + sx) * 4;
                    let rgb = [px[o], px[o + 1], px[o + 2]].map(|v| v as f32 / 255.0);
                    for k in 0..3 {
                        s[k] += rgb[k];
                    }
                    m = m.max(luma(rgb));
                    c += 1.0;
                }
            }
            avg[y * w + x] = s.map(|v| v / c.max(1.0));
            peak[y * w + x] = m;
        }
    }
    let lum: Vec<f32> = avg.iter().map(|&c| luma(c)).collect();
    let mut local = lum.clone();
    blur(&mut local, w, h, (w / 50).max(2), 2);
    // Brightest point nearby: real flames have a yellow-white core somewhere close.
    let mut core = peak.clone();
    max_filter(&mut core, w, h, (w / 200).max(2));
    let sg = |i: usize, ch: usize| seg.map(|s| s[i * 4 + ch] as f32 / 255.0).unwrap_or(0.0);

    let mut fire = vec![0f32; n];
    let mut glow = vec![0f32; n];
    let mut fxmap = vec![0u8; n * 4];
    for i in 0..n {
        let [r, g, b] = avg[i];
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        let hue = hue_deg(r, g, b);
        let (sky, people) = (sg(i, 0), sg(i, 3));
        // Hot colours: red-orange to yellow, saturated, bright, standing out from around them.
        let hot = smooth(0.0, 8.0, hue) * smooth(55.0, 38.0, hue) + smooth(345.0, 355.0, hue);
        let stands_out = smooth(1.2, 1.8, lum[i] / local[i].max(0.02));
        // Real flames are bright (orange roof tiles or skin are not) and glow against
        // their surroundings.
        fire[i] = (hot * smooth(0.5, 0.75, sat) * smooth(0.6, 0.9, mx) * smooth(0.4, 0.58, lum[i]) * smooth(0.7, 0.9, core[i]) * stands_out
            * (1.0 - 0.85 * people)
            * (1.0 - 0.6 * sky))
            .clamp(0.0, 1.0);
        // Lights: a bright peak well above the neighbourhood.
        // (Not on people: a glint in someone's eye looks wrong.)
        glow[i] = (smooth(0.9, 0.99, peak[i]) * smooth(0.35, 0.6, peak[i] - local[i]) * (1.0 - fire[i]) * (1.0 - 0.8 * people)).clamp(0.0, 1.0);
    }
    let mut water = vec![0f32; n];
    let mut fire_s = fire.clone();
    blur(&mut fire_s, w, h, 1, 1);
    for i in 0..n {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        // Stars only in a dark sky; elsewhere the sky channel just lets clouds drift.
        fxmap[i * 4] = q(fire_s[i]);
        // Twinkle: lights, plus specks in a sky that is actually dark (a bright misty sky
        // has none).
        let star = smooth(0.5, 0.8, sg(i, 0)) * smooth(0.3, 0.12, local[i]) * smooth(0.08, 0.25, peak[i] - local[i]);
        fxmap[i * 4 + 1] = q(glow[i].max(star));
        // Grass gets mistaken for water; real water is rarely a saturated green.
        let [r, g, b] = avg[i];
        let green = smooth(65.0, 85.0, hue_deg(r, g, b)) * smooth(165.0, 145.0, hue_deg(r, g, b));
        let sat = (r.max(g).max(b) - r.min(g).min(b)) / r.max(g).max(b).max(1e-3);
        water[i] = smooth(0.5, 0.8, sg(i, 1)) * (1.0 - green * smooth(0.25, 0.45, sat));
        fxmap[i * 4 + 2] = q(water[i]);
        fxmap[i * 4 + 3] = q(smooth(0.4, 0.75, sg(i, 0)));
    }

    // Emitters: the strongest well-separated spots of each kind.
    let depth_at = |i: usize| cards.get(i * 4).map(|&v| v as f32 / 255.0).unwrap_or(0.5);
    let color_at = |i: usize| {
        let c = Vec3::from(avg[i].map(srgb_to_linear));
        c / c.max_element().max(1e-3)
    };
    let mut emitters = Vec::new();
    let mut pick = |kind: EmitKind, score: &dyn Fn(usize) -> f32, min: f32, max_n: usize, spacing: usize| {
        let mut cand: Vec<(f32, usize)> = (0..n).map(|i| (score(i), i)).filter(|&(s, _)| s >= min).collect();
        cand.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut chosen: Vec<usize> = Vec::new();
        for (s, i) in cand {
            if chosen.len() >= max_n {
                break;
            }
            let (x, y) = (i % w, i / w);
            if chosen.iter().all(|&j| (j % w).abs_diff(x).max((j / w).abs_diff(y)) >= spacing) {
                chosen.push(i);
                emitters.push(Emitter {
                    kind,
                    uv: Vec2::new((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32),
                    depth: depth_at(i),
                    strength: s,
                    color: color_at(i),
                });
            }
        }
    };
    let unit = (w / 100).max(1);
    // Ignore the bottom of the image: it fades out behind the shelf.
    let visible = |i: usize| smooth(0.92, 0.75, (i / w) as f32 / h as f32);
    pick(EmitKind::Ember, &|i| fire_s[i] * visible(i), 0.45, 16, unit * 3);
    pick(EmitKind::Glint, &|i| glow[i] * (1.0 - sg(i, 0)) * visible(i), 0.5, 6, unit * 5);
    pick(
        EmitKind::Glint,
        &|i| water[i] * smooth(0.15, 0.35, peak[i] - local[i]) * smooth(0.6, 0.85, peak[i]) * visible(i),
        0.4,
        8,
        unit * 3,
    );
    pick(
        EmitKind::Star,
        &|i| smooth(0.5, 0.8, sg(i, 0)) * smooth(0.3, 0.12, local[i]) * smooth(0.1, 0.3, peak[i] - local[i]),
        0.3,
        24,
        unit,
    );
    let foliage_edge = |i: usize| {
        // The top edge of a canopy, not where trees run off the top of the picture.
        if i < w * (h / 30).max(3) {
            return 0.0;
        }
        let above = sg(i - w * 2, 2);
        smooth(0.5, 0.8, sg(i, 2)) * smooth(0.6, 0.2, above) * visible(i)
    };
    pick(EmitKind::Leaf, &foliage_edge, 0.3, 12, unit * 4);
    // Motes drift around the lights.
    let lights: Vec<Emitter> = emitters.iter().filter(|e| e.kind == EmitKind::Glint).take(6).cloned().collect();
    emitters.extend(lights.into_iter().map(|e| Emitter { kind: EmitKind::Mote, ..e }));

    Analysis { fxmap, emitters }
}

fn max_filter(v: &mut [f32], w: usize, h: usize, r: usize) {
    let mut tmp = vec![0.0f32; v.len()];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            tmp[y * w + x] = v[y * w + a..=y * w + b].iter().copied().fold(0.0, f32::max);
        }
    }
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
            v[y * w + x] = (a..=b).map(|yy| tmp[yy * w + x]).fold(0.0, f32::max);
        }
    }
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn hue_deg(r: f32, g: f32, b: f32) -> f32 {
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    if d <= 1e-5 {
        return 0.0;
    }
    let h = if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h * 60.0
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// smoothstep that also accepts e0 > e1 (falling edge).
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn blur(v: &mut [f32], w: usize, h: usize, r: usize, passes: usize) {
    let mut tmp = vec![0.0f32; v.len()];
    for _ in 0..passes {
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
                tmp[y * w + x] = v[y * w + a..=y * w + b].iter().sum::<f32>() / (b - a + 1) as f32;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
                v[y * w + x] = (a..=b).map(|yy| tmp[yy * w + x]).sum::<f32>() / (b - a + 1) as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fire_in_the_dark_sheds_embers() {
        let (w, h) = (100u32, 50u32);
        let px: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                // Orange flames around a yellow-white core.
                if (62..64).contains(&x) && (22..24).contains(&y) {
                    [255, 235, 170, 255]
                } else if (60..66).contains(&x) && (20..26).contains(&y) {
                    [255, 140, 30, 255]
                } else {
                    [20, 25, 35, 255]
                }
            })
            .collect();
        let cards = vec![128u8; (w * h * 4) as usize];
        let a = analyse(w, h, &px, w, h, &cards, None);
        let embers: Vec<_> = a.emitters.iter().filter(|e| e.kind == EmitKind::Ember).collect();
        assert!(!embers.is_empty());
        assert!(embers.iter().all(|e| (0.58..0.68).contains(&e.uv.x) && (0.38..0.54).contains(&e.uv.y)), "{embers:?}");
        // The dark background makes nothing.
        assert!(a.emitters.iter().all(|e| e.uv.x > 0.5));
    }

    #[test]
    fn skin_and_grey_are_not_fire() {
        let (w, h) = (40u32, 20u32);
        // Warm skin tone (low saturation) and neutral grey.
        let px: Vec<u8> = (0..w * h).flat_map(|i| if i % w < 20 { [225, 175, 145, 255] } else { [128, 128, 128, 255] }).collect();
        let cards = vec![128u8; (w * h * 4) as usize];
        let a = analyse(w, h, &px, w, h, &cards, None);
        assert!(a.emitters.iter().all(|e| e.kind != EmitKind::Ember), "{:?}", a.emitters);
    }
}

/// `cargo test dump_scene -- --ignored` draws what the analysis found over each hero (set
/// `GDASH_DEPTH_OUT` and `GDASH_DEPTH_IDS`): ember orange, glint white, star blue, leaf green.
#[cfg(test)]
#[test]
#[ignore]
fn dump_scene() {
    let out = std::path::PathBuf::from(std::env::var("GDASH_DEPTH_OUT").expect("GDASH_DEPTH_OUT"));
    let ids = std::env::var("GDASH_DEPTH_IDS").unwrap_or_default();
    for id in ids.split(',').filter(|s| !s.is_empty()) {
        let Some(p) = crate::art::cached(id, crate::art::ArtKind::Hero) else { continue };
        let img = image::open(p).unwrap().to_rgba8();
        let Some((w, h, d)) = super::depth::load_override(id) else { continue };
        let seg = load_segment(id, w, h);
        let people = subjects(seg.as_deref());
        let cards = super::depth::layered(w, h, &d, people.as_deref());
        let a = analyse(img.width(), img.height(), img.as_raw(), w, h, &cards, seg.as_deref());
        let tw = 960u32;
        let th = tw * img.height() / img.width();
        let mut thumb = image::imageops::thumbnail(&img, tw, th);
        // Tint the effect regions: fire red, lights yellow, water cyan, sky blue.
        for y in 0..th {
            for x in 0..tw {
                let i = ((y * h / th) * w + x * w / tw) as usize * 4;
                let [f, g, wa, sk] = [a.fxmap[i], a.fxmap[i + 1], a.fxmap[i + 2], a.fxmap[i + 3]].map(|v| v as f32 / 255.0);
                let px = thumb.get_pixel_mut(x, y);
                let mut c = [px[0] as f32, px[1] as f32, px[2] as f32];
                for (k, (amt, tint)) in [(f, [255.0, 40.0, 0.0]), (g, [255.0, 255.0, 0.0]), (wa * 0.5, [0.0, 255.0, 255.0]), (sk * 0.3, [60.0, 60.0, 255.0])].into_iter().enumerate() {
                    let _ = k;
                    for j in 0..3 {
                        c[j] = c[j] * (1.0 - amt) + tint[j] * amt;
                    }
                }
                *px = image::Rgba([c[0] as u8, c[1] as u8, c[2] as u8, 255]);
            }
        }
        for e in &a.emitters {
            let col = match e.kind {
                EmitKind::Ember => [255, 140, 0, 255],
                EmitKind::Glint => [255, 255, 255, 255],
                EmitKind::Star => [80, 160, 255, 255],
                EmitKind::Leaf => [0, 255, 0, 255],
                EmitKind::Mote => continue,
            };
            let (cx, cy) = ((e.uv.x * tw as f32) as i32, (e.uv.y * th as f32) as i32);
            for dy in -3..=3 {
                for dx in -3..=3 {
                    let (x, y) = (cx + dx, cy + dy);
                    if (dx * dx + dy * dy) <= 9 && x >= 0 && y >= 0 && (x as u32) < tw && (y as u32) < th {
                        thumb.put_pixel(x as u32, y as u32, image::Rgba(col));
                    }
                }
            }
        }
        thumb.save(out.join(format!("{id}-scene.png"))).unwrap();
        let counts: Vec<String> = [EmitKind::Ember, EmitKind::Glint, EmitKind::Star, EmitKind::Leaf]
            .iter()
            .map(|k| format!("{k:?}={}", a.emitters.iter().filter(|e| e.kind == *k).count()))
            .collect();
        println!("{id}: {}", counts.join(" "));
    }
}
