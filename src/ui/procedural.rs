//! Generated cover art for games with no box art.
//!
//! Dark ink, ribbons of the game's own colour drifting across, a few bokeh discs and
//! grain — the vocabulary of the showcase background, at card scale, seeded by the
//! game's id. A game without art still looks like part of the library, and the shelf
//! still has something living to reflect.

use bevy::prelude::*;

use super::fx;

/// Cover size, 2:3 like the shelf tiles and grid cards that show it.
pub const W: u32 = 340;
pub const H: u32 = 510;

fn f(h: u64, shift: u32) -> f32 {
    ((h >> shift) & 0xffff) as f32 / 65535.0
}

fn smooth01(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn hash2(ix: i32, iy: i32, seed: u64) -> f32 {
    let mut n = (ix as i64 as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ (iy as i64 as u64).wrapping_mul(0xC2B2AE3D27D4EB4F)
        ^ seed;
    n = n.wrapping_mul(0x27D4EB2F165667C5);
    n ^= n >> 33;
    n = n.wrapping_mul(0x85EBCA6B2EE58A0F);
    n ^= n >> 29;
    (n >> 40) as f32 / 16_777_216.0
}

fn vnoise(x: f32, y: f32, seed: u64) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (smooth01(x - xi), smooth01(y - yi));
    let (x0, y0) = (xi as i32, yi as i32);
    let a = hash2(x0, y0, seed);
    let b = hash2(x0 + 1, y0, seed);
    let c = hash2(x0, y0 + 1, seed);
    let d = hash2(x0 + 1, y0 + 1, seed);
    let ab = a + (b - a) * fx;
    let cd = c + (d - c) * fx;
    ab + (cd - ab) * fy
}

fn fbm(x: f32, y: f32, seed: u64) -> f32 {
    let mut v = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    for o in 0..4u64 {
        v += amp * vnoise(x * freq, y * freq, seed ^ (o + 1).wrapping_mul(0x9E3779B97F4A7C15));
        amp *= 0.5;
        freq *= 2.07;
    }
    v
}

/// The cover for a game: RGBA8 bytes, `W * H * 4` long.
pub fn cover(id: &str, accent: Color) -> Vec<u8> {
    let h = fx::hash(id);
    let hsv: Hsva = accent.into();
    // Push the colour to its vivid end, the way `signature_color` lifts real art.
    let glow = Color::from(Hsva::new(hsv.hue, hsv.saturation.clamp(0.55, 0.9), 1.0, 1.0)).to_srgba();
    let g = [glow.red, glow.green, glow.blue];
    // A second hue a little apart, so the two ribbons never read as one.
    let hue2 = hsv.hue + (f(h, 3) - 0.5) * 70.0;
    let glow2 = Color::from(Hsva::new(hue2, 0.7, 1.0, 1.0)).to_srgba();
    let g2 = [glow2.red, glow2.green, glow2.blue];

    let (wf, hf) = (W as f32, H as f32);
    let mut px = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        let v = y as f32 / hf;
        for x in 0..W {
            let u = x as f32 / wf;
            let mut col = [
                0.105 - 0.085 * v,
                0.135 - 0.105 * v,
                0.175 - 0.135 * v,
            ];

            // Ink smoke: low, soft, keeps the card from being flat.
            let n = fbm(u * 3.8, v * 5.6, h);
            let smoke = (n - 0.47) * 0.10;
            for c in col.iter_mut() {
                *c += smoke;
            }

            // Two ribbons of light, soft-edged and drifting apart.
            for (k, tint) in [g, g2].iter().enumerate() {
                let phase = f(h, 10 + k as u32 * 5) * 6.283;
                let slope = (f(h, 20 + k as u32 * 5) - 0.5) * 1.8;
                let freq = 1.4 + f(h, 30 + k as u32 * 5) * 1.4;
                let wave = (u * freq + v * slope + phase).sin();
                let band = (0.5 + 0.5 * wave).powi(9);
                let strength = if k == 0 { 0.30 } else { 0.16 };
                for (i, c) in col.iter_mut().enumerate() {
                    // Screen blend: light on ink, never mud.
                    *c = 1.0 - (1.0 - *c) * (1.0 - tint[i] * band * strength);
                }
            }

            // Bokeh: a few soft discs, half way to white, out of focus.
            let discs = 4 + (h % 3) as u32;
            for k in 0..discs {
                // Each disc reads its own bit-stream of the seed.
                let hk = h ^ (k as u64).wrapping_mul(0x9E3779B97F4A7C15);
                let cx = f(hk, 40) * wf;
                let cy = f(hk, 20) * hf;
                let rad = 10.0 + 22.0 * f(hk, 30);
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                let disc = 1.0 - smooth01((d / rad).min(1.0));
                let alpha = 0.05 + 0.07 * f(hk, 50);
                let tint = tint_at(g, g2, k);
                for (i, c) in col.iter_mut().enumerate() {
                    *c += (tint[i] * 0.45 + 0.55) * disc * alpha;
                }
            }

            // Vignette, and a dark last third where the title will sit.
            let (dx, dy) = ((u - 0.5) * 2.0, (v - 0.5) * 2.0);
            let edge = smooth01((((dx * dx + dy * dy).sqrt()) - 0.55) / 0.75);
            let scrim = smooth01(((v - 0.60) / 0.40).clamp(0.0, 1.0));
            for c in col.iter_mut() {
                *c *= (1.0 - 0.22 * edge) * (1.0 - 0.60 * scrim);
                // Grain.
                *c += (hash2(x as i32, y as i32, h) - 0.5) * (2.5 / 255.0);
                px.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            px.push(255);
        }
    }
    px
}

fn tint_at(g: [f32; 3], g2: [f32; 3], k: u32) -> [f32; 3] {
    if k % 2 == 0 { g } else { g2 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_id_same_art() {
        let a = cover("pc-my game", super::super::theme::PC);
        let b = cover("pc-my game", super::super::theme::PC);
        assert_eq!(a, b);
        assert_eq!(a.len(), (W * H * 4) as usize);
    }

    #[test]
    fn different_games_differ() {
        let a = cover("steam-1", super::super::theme::STEAM);
        let b = cover("steam-2", super::super::theme::STEAM);
        assert_ne!(a, b);
    }

    #[test]
    fn bottom_is_darker_than_top() {
        let px = cover("x", super::super::theme::ALL);
        let lum = |y: u32| -> f32 {
            let mut s = 0.0;
            for x in 0..W {
                let i = ((y * W + x) * 4) as usize;
                s += px[i] as f32 + px[i + 1] as f32 + px[i + 2] as f32;
            }
            s
        };
        assert!(lum(0) > lum(H - 1), "top row should be brighter than the last");
    }

    /// Renders a contact sheet of generated covers for eyeballing (run with
    /// `cargo test render -- --nocapture`, then look at target/procedural-covers.png).
    #[test]
    fn render_contact_sheet() {
        let cases = [
            ("steam-570", super::super::theme::STEAM),
            ("pc-hollow knight", super::super::theme::PC),
            ("xbox-jetsetradio", super::super::theme::XBOX),
            ("x360-oblivion", super::super::theme::XBOX360),
            ("pc-grey game", super::super::theme::SYSTEM),
            ("steam-730", super::super::theme::STEAM),
        ];
        let cols = 3;
        let rows = cases.len().div_ceil(cols);
        let (cw, ch) = (W * cols as u32, H * rows as u32);
        let mut sheet = vec![8u8; (cw * ch * 4) as usize];
        for (k, (id, accent)) in cases.iter().enumerate() {
            let px = cover(id, *accent);
            let (cx, cy) = ((k % cols) as u32, (k / cols) as u32);
            for y in 0..H {
                for x in 0..W {
                    let src = ((y * W + x) * 4) as usize;
                    let dst = (((cy * H + y) * cw + cx * W + x) * 4) as usize;
                    sheet[dst..dst + 4].copy_from_slice(&px[src..src + 4]);
                }
            }
        }
        let path = std::path::Path::new("target").join("procedural-covers.png");
        image::save_buffer(&path, sheet.as_slice(), cw, ch, image::ColorType::Rgba8).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
