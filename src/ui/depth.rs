//! A cheap monocular "depth" guess for hero art, so the background can move like a 2.5D
//! photo. Near = 1, far = 0. It combines cues that hold for most key art: lower in the frame
//! is nearer, detailed regions are subjects, bright flat regions near the top are sky. The
//! result is heavily smoothed: soft depth gives gentle, convincing parallax; sharp wrong
//! depth tears.
//!
//! A real depth map (for example from a depth-estimation model) can replace this per game:
//! drop a grayscale `depth.png` (white = near) into the game's art folder.

/// Depth map width; height follows the image aspect.
const W: usize = 192;

pub fn estimate(w: u32, h: u32, px: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w, h) = (w as usize, h as usize);
    let dh = ((h as f32 / w as f32) * W as f32).round().clamp(1.0, 512.0) as usize;
    let n = W * dh;
    let mut lum = vec![0.0f32; n];
    let mut sat = vec![0.0f32; n];

    // Downsample: average a 3x3 grid of samples inside each block.
    for y in 0..dh {
        for x in 0..W {
            let (mut l, mut s) = (0.0, 0.0);
            for j in 0..3 {
                for i in 0..3 {
                    let sx = (((x as f32 + (i as f32 + 0.5) / 3.0) / W as f32) * w as f32) as usize;
                    let sy = (((y as f32 + (j as f32 + 0.5) / 3.0) / dh as f32) * h as f32) as usize;
                    let o = (sy.min(h - 1) * w + sx.min(w - 1)) * 4;
                    let [r, g, b] = [px[o], px[o + 1], px[o + 2]].map(|c| c as f32 / 255.0);
                    let max = r.max(g).max(b);
                    l += 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    s += if max > 0.0 { (max - r.min(g).min(b)) / max } else { 0.0 };
                }
            }
            lum[y * W + x] = l / 9.0;
            sat[y * W + x] = s / 9.0;
        }
    }

    // Local detail: gradient magnitude, spread out so whole subjects count, not just edges.
    let at = |v: &[f32], x: isize, y: isize| v[(y.clamp(0, dh as isize - 1) as usize) * W + x.clamp(0, W as isize - 1) as usize];
    let mut detail = vec![0.0f32; n];
    for y in 0..dh as isize {
        for x in 0..W as isize {
            let gx = at(&lum, x + 1, y) - at(&lum, x - 1, y);
            let gy = at(&lum, x, y + 1) - at(&lum, x, y - 1);
            detail[y as usize * W + x as usize] = (gx * gx + gy * gy).sqrt();
        }
    }
    blur(&mut detail, W, dh, 4, 3);
    let mut sorted = detail.clone();
    sorted.sort_by(f32::total_cmp);
    let p95 = sorted[(n as f32 * 0.95) as usize].max(1e-4);

    let mut depth = vec![0.0f32; n];
    for y in 0..dh {
        let v = y as f32 / (dh - 1).max(1) as f32;
        for x in 0..W {
            let k = y * W + x;
            let d = (detail[k] / p95).min(1.0);
            let sky = (1.0 - d) * smoothstep(0.55, 0.05, v) * smoothstep(0.35, 0.85, lum[k]);
            depth[k] = 0.15 + 0.45 * smoothstep(0.15, 1.0, v) + 0.35 * d + 0.1 * sat[k] - 0.5 * sky;
        }
    }
    blur(&mut depth, W, dh, 6, 3);

    let (lo, hi) = depth.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let span = (hi - lo).max(1e-4);
    let out = depth
        .iter()
        .flat_map(|&v| {
            let g = (((v - lo) / span) * 255.0).round() as u8;
            [g, g, g, 255]
        })
        .collect();
    (W as u32, dh as u32, out)
}

/// The image's centre of interest, in UV: the centroid of its nearest parts (depth^4
/// weighted, so subjects dominate), kept away from the edges.
pub fn focal(w: u32, h: u32, rgba: &[u8]) -> (f32, f32) {
    let (mut sx, mut sy, mut sw) = (0.0f64, 0.0f64, 0.0f64);
    let step = ((w * h) as usize / 20000).max(1);
    for i in (0..(w * h) as usize).step_by(step) {
        let d = rgba[i * 4] as f64 / 255.0;
        let wt = d * d * d * d;
        sx += (i % w as usize) as f64 * wt;
        sy += (i / w as usize) as f64 * wt;
        sw += wt;
    }
    if sw <= 0.0 {
        return (0.5, 0.4);
    }
    let fx = (sx / sw / w.max(1) as f64) as f32;
    let fy = (sy / sw / h.max(1) as f64) as f32;
    (fx.clamp(0.2, 0.8), fy.clamp(0.2, 0.75))
}

/// Turns a smooth depth map into rigid "cards" for the parallax: every connected subject
/// (anything nearer than the background, plus anything `subjects` marks, e.g. people found by
/// segmentation) gets one flat depth, so it moves as a single piece and never bends. The
/// background keeps only a very soft depth, and every seam between a subject and what's behind
/// it is pushed outward into the background, where a little stretching of clouds or scenery
/// goes unnoticed.
pub fn layered(w: u32, h: u32, rgba: &[u8], subjects: Option<&[f32]>) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let n = w * h;
    let d: Vec<f32> = rgba.chunks_exact(4).map(|p| p[0] as f32 / 255.0).collect();
    let t = otsu(&d).clamp(0.25, 0.7);
    let fg: Vec<bool> = (0..n).map(|i| d[i] > t || subjects.is_some_and(|s| s[i] > 0.5)).collect();

    // Label connected subjects; each becomes one card at its mean depth.
    const NONE: u32 = u32::MAX;
    const SPECK: u32 = u32::MAX - 1;
    let mut label = vec![NONE; n];
    let mut cards: Vec<f32> = Vec::new();
    let min_area = (n / 700).max(4);
    let mut stack = Vec::new();
    for start in 0..n {
        if !fg[start] || label[start] != NONE {
            continue;
        }
        let id = cards.len() as u32;
        let mut members = Vec::new();
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            members.push(i);
            let (x, y) = (i % w, i / w);
            let mut near = [None; 4];
            if x > 0 { near[0] = Some(i - 1); }
            if x + 1 < w { near[1] = Some(i + 1); }
            if y > 0 { near[2] = Some(i - w); }
            if y + 1 < h { near[3] = Some(i + w); }
            for j in near.into_iter().flatten() {
                if fg[j] && label[j] == NONE {
                    label[j] = id;
                    stack.push(j);
                }
            }
        }
        let mean = members.iter().map(|&i| d[i]).sum::<f32>() / members.len() as f32;
        if members.len() < min_area {
            // Specks aren't worth a card; they ride along with the background.
            for i in members {
                label[i] = SPECK;
            }
        }
        cards.push(mean.max(t));
    }
    let card_of = |i: usize| (label[i] < SPECK).then(|| cards[label[i] as usize]);

    // Background: very soft depth, always behind the subjects.
    let mut bg = d.clone();
    blur(&mut bg, w, h, (w / 60).max(2), 3);
    for v in &mut bg {
        *v = v.min(t);
    }

    // Grow every card outward so the seam sits in the background, then soften it there.
    let reach = (w / 90).max(3);
    let mut grown: Vec<f32> = (0..n).map(|i| card_of(i).unwrap_or(-1.0)).collect();
    max_filter(&mut grown, w, h, reach);
    let mut out: Vec<f32> = (0..n).map(|i| card_of(i).unwrap_or(grown[i].max(bg[i]))).collect();
    blur(&mut out, w, h, (reach / 2).max(1), 2);
    for i in 0..n {
        if let Some(c) = card_of(i) {
            out[i] = c;
        }
    }
    out.iter()
        .flat_map(|&v| {
            let g = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            [g, g, g, 255]
        })
        .collect()
}

/// Otsu's threshold on 0..1 values: the split that best separates two groups.
fn otsu(v: &[f32]) -> f32 {
    let mut hist = [0u32; 64];
    for &x in v {
        hist[(x.clamp(0.0, 1.0) * 63.0).round() as usize] += 1;
    }
    let total = v.len().max(1) as f64;
    let sum_all: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut w0, mut sum0, mut best, mut best_t) = (0.0f64, 0.0f64, -1.0f64, 32usize);
    for (i, &c) in hist.iter().enumerate() {
        w0 += c as f64;
        sum0 += i as f64 * c as f64;
        let w1 = total - w0;
        if w0 == 0.0 || w1 == 0.0 {
            continue;
        }
        let (m0, m1) = (sum0 / w0, (sum_all - sum0) / w1);
        let between = w0 * w1 * (m0 - m1) * (m0 - m1);
        if between > best {
            best = between;
            best_t = i;
        }
    }
    (best_t as f32 + 0.5) / 63.0
}

/// Square max filter (separable), radius `r`.
fn max_filter(v: &mut [f32], w: usize, h: usize, r: usize) {
    let mut tmp = vec![0.0f32; v.len()];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            tmp[y * w + x] = v[y * w + a..=y * w + b].iter().copied().fold(f32::MIN, f32::max);
        }
    }
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
            v[y * w + x] = (a..=b).map(|yy| tmp[yy * w + x]).fold(f32::MIN, f32::max);
        }
    }
}

/// Loads a user/model supplied depth map for hero `variant` (depth.png / depth2.png /
/// depth3.png), if the game has one.
pub fn load_override_at(id: &str, variant: u32) -> Option<(u32, u32, Vec<u8>)> {
    let name = match variant { 1 => "depth2.png", 2 => "depth3.png", _ => "depth.png" };
    let p = crate::art::game_dir(id).join(name);
    let img = image::open(p).ok()?.to_luma8();
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw().into_iter().flat_map(|g| [g, g, g, 255]).collect()))
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Repeated separable box blur (approximates a Gaussian).
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
    fn layers_flatten_a_ramp() {
        let (w, h) = (60u32, 4u32);
        let px: Vec<u8> = (0..w * h).flat_map(|i| {
            let g = ((i % w) * 255 / (w - 1)) as u8;
            [g, g, g, 255]
        }).collect();
        let out = layered(w, h, &px, None);
        // The near part is one flat card.
        let near: Vec<u8> = (w - 25..w).map(|x| out[((w + x) * 4) as usize]).collect();
        assert!(near.iter().all(|&v| v == near[0]), "{near:?}");
    }

    #[test]
    fn a_subject_is_one_rigid_card() {
        // A "person" whose own depth varies a lot (face nearer than shoulders), on a far sky.
        let (w, h) = (120u32, 80u32);
        let px: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                let inside = (40..80).contains(&x) && y > 20;
                let g = if inside { 150 + (y * 100 / h) as u8 } else { 20 };
                [g, g, g, 255]
            })
            .collect();
        let out = layered(w, h, &px, None);
        let at = |x: u32, y: u32| out[((y * w + x) * 4) as usize];
        let v = at(60, 30);
        for (x, y) in [(41, 21), (79, 79), (60, 70), (45, 50)] {
            assert_eq!(at(x, y), v, "({x},{y})");
        }
        // The background right next to it is pulled forward too (the seam is outside).
        assert!(at(38, 50) > 120, "{}", at(38, 50));
        assert!(at(5, 5) < 60, "{}", at(5, 5));
    }

    #[test]
    fn focal_follows_the_near_subject() {
        // Near blob on the right third.
        let (w, h) = (90u32, 30u32);
        let px: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let near = (i % w) > 60 && (i / w) > 10 && (i / w) < 20;
                let g = if near { 255 } else { 20 };
                [g, g, g, 255]
            })
            .collect();
        let (fx, fy) = focal(w, h, &px);
        assert!(fx > 0.7 && (0.4..0.6).contains(&fy), "{fx} {fy}");
    }

    #[test]
    fn sky_is_far_and_ground_is_near() {
        // Bright flat sky on top, busy dark checker "ground" below.
        let (w, h) = (320u32, 180u32);
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let c = if y < h / 2 { 230 } else if (x / 4 + y / 4) % 2 == 0 { 30 } else { 140 };
                px.extend([c, c, c, 255]);
            }
        }
        let (dw, dh, d) = estimate(w, h, &px);
        assert_eq!(dw, 192);
        let at = |x: u32, y: u32| d[((y * dw + x) * 4) as usize];
        assert!(at(dw / 2, dh / 8) < 60, "sky {}", at(dw / 2, dh / 8));
        assert!(at(dw / 2, dh - 4) > 190, "ground {}", at(dw / 2, dh - 4));
    }
}

/// `cargo test dump_depth -- --ignored` writes depth maps for cached heroes to `GDASH_DEPTH_OUT`.
#[cfg(test)]
#[test]
#[ignore]
fn dump_depth() {
    let out = std::path::PathBuf::from(std::env::var("GDASH_DEPTH_OUT").expect("GDASH_DEPTH_OUT"));
    let ids = std::env::var("GDASH_DEPTH_IDS").unwrap_or_default();
    for id in ids.split(',').filter(|s| !s.is_empty()) {
        let Some(p) = crate::art::cached(id, crate::art::ArtKind::Hero) else { continue };
        let img = image::open(p).unwrap().to_rgba8();
        let (w, h, d) = load_override_at(id, 0).unwrap_or_else(|| estimate(img.width(), img.height(), img.as_raw()));
        let cards = layered(w, h, &d, None);
        image::RgbaImage::from_raw(w, h, d.to_vec()).unwrap().save(out.join(format!("{id}-depth.png"))).unwrap();
        image::RgbaImage::from_raw(w, h, cards.to_vec()).unwrap().save(out.join(format!("{id}-cards.png"))).unwrap();
        image::imageops::thumbnail(&img, 480, 480 * img.height() / img.width()).save(out.join(format!("{id}-hero.png"))).unwrap();
    }
}

