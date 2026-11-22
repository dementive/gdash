//! On-device ML improvements to cached artwork, run once per image:
//!
//! - **Depth**: Depth Anything 3 (small) turns each hero image into `depth.png`, which drives
//!   the moving background (white = near).
//! - **Segmentation**: SegFormer-B0 (ADE20K) finds sky, water, foliage and people, saved as
//!   `segment.png` (R sky, G water, B foliage, A people/animals). The dashboard uses it to make
//!   effects come out of the picture itself (stars in the sky, glints on water, falling leaves)
//!   and to keep people rigid when the image moves.
//! - **Super-resolution**: Real-ESRGAN General x4v3 upscales low-resolution heroes (Steam's
//!   1920-wide art, old console screenshots) so they hold up on 4K screens. The original is
//!   kept as `hero-original.*`.
//!
//! Models are downloaded on first use into the data folder and verified against pinned
//! SHA-256 hashes. Inference runs on the CPU. The dashboard runs this as a separate
//! low-priority process (`gdash --enhance-art`) so the models' memory never stays in the UI.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use image::{DynamicImage, GenericImageView, RgbImage};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use sha2::{Digest, Sha256};

use crate::art::{self, ArtKind};
use crate::library::Game;
use crate::paths;

/// Depth Anything V3 Small (Apache-2.0), ONNX export by onnx-community. Its weights live
/// in a companion file the graph references by name (see `DEPTH_DATA_FILE`).
const DEPTH_URL: &str = "https://huggingface.co/onnx-community/depth-anything-v3-small/resolve/0b6a7f3bf5595f9950b91389e0da3a0de130324c/onnx/model.onnx";
const DEPTH_SHA: &str = "396008798244a074297fd88e450433b1357fc687f534939375c804ded86e7b2a";
const DEPTH_FILE: &str = "depth-anything-v3-small.onnx";
/// The graph's external weights. The file name must stay exactly what the graph
/// references, or it will not load — hence the generic name, pinned by hash like the rest.
const DEPTH_DATA_URL: &str = "https://huggingface.co/onnx-community/depth-anything-v3-small/resolve/0b6a7f3bf5595f9950b91389e0da3a0de130324c/onnx/model.onnx_data";
const DEPTH_DATA_SHA: &str = "802bb24741e67f5bb2b369fc64d40afe11439cc895d676d658d65cfb75c9860f";
const DEPTH_DATA_FILE: &str = "model.onnx_data";
/// Name of the depth model recorded in each game's art folder when its `depth.png` was
/// generated, so a model swap regenerates old maps. A `depth.png` without the marker is
/// the user's own and always wins.
const DEPTH_MODEL_FILE: &str = "depth.model";
const DEPTH_MODEL: &str = "depth-anything-v3-small";

/// SegFormer-B0 fine-tuned on ADE20K (150 classes), ONNX export by Xenova.
const SEG_URL: &str = "https://huggingface.co/Xenova/segformer-b0-finetuned-ade-512-512/resolve/d3e5499fa8701ff0453ca940a8dfeae39b2f1504/onnx/model.onnx";
const SEG_SHA: &str = "3e5c18a4be395f16646438d54c42377ddc202edfa33d5eced0c9506de75c44c2";
const SEG_FILE: &str = "segformer-b0-ade.onnx";
const SEG_IN: u32 = 512;
/// ADE20K classes grouped into the channels of `segment.png`.
const SEG_GROUPS: [&[usize]; 4] = [
    &[2],                                // sky
    &[21, 26, 60, 104, 109, 113, 128],   // water, sea, river, fountain, pool, waterfall, lake
    &[4, 17, 66, 72],                    // tree, plant, flower, palm
    &[12, 126],                          // person, animal
];

/// Real-ESRGAN General x4v3 (BSD-3-Clause), ONNX export by Qualcomm AI Hub. Fixed 128x128 input.
const SR_URL: &str = "https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/real_esrgan_general_x4v3/releases/v0.66.0/real_esrgan_general_x4v3-onnx-float.zip";
const SR_SHA: &str = "fc094329b24a00c936ebc1cc4c51bc59633e7dc36f4a553344342171cbd3c7cf";
const SR_FILE: &str = "real_esrgan_general_x4v3.onnx";
const SR_TILE: usize = 128;
const SR_SCALE: usize = 4;
/// Input pixels each tile overlaps its neighbours by; seams are hidden inside the overlap.
const SR_OVERLAP: usize = 16;

/// Heroes shorter than this get upscaled (Steam's 2x heroes are 1240 tall and are left alone).
const SR_BELOW_H: u32 = 1000;
/// Depth inference resolution (short side, a multiple of 14). 518 is DA3's native patch
/// grid (37 x 14); wider heroes keep their aspect ratio.
const DEPTH_SHORT: u32 = 518;

/// What changed for a game.
pub struct Enhanced {
    pub id: String,
    pub upscaled: bool,
    pub depth: bool,
    pub segment: bool,
}

fn models_dir() -> PathBuf {
    paths::data_dir().join("models")
}

/// Work still to do for this game, cheaply checked from files on disk.
#[derive(Clone, Copy, Default, PartialEq)]
struct Todo {
    sr: bool,
    depth: bool,
    segment: bool,
}

impl Todo {
    fn any(self) -> bool {
        self.sr || self.depth || self.segment
    }
}

fn todo(game: &Game) -> Todo {
    let Some(hero) = art::cached(&game.id, ArtKind::Hero) else { return Todo::default() };
    let dir = art::game_dir(&game.id);
    let broken = dir.join("depth.none").exists();
    Todo {
        sr: !dir.join("hero.sr").exists() && image::image_dimensions(&hero).is_ok_and(|(_, h)| h < SR_BELOW_H),
        depth: !depth_done(&dir) && !broken,
        segment: !dir.join("segment.png").exists() && !broken,
    }
}

/// True when the depth map on disk needs no regeneration: one exists, and either the
/// current model wrote it (see the marker) or no model did — a hand-dropped `depth.png`
/// always wins, as the README promises. Maps by an older model regenerate.
fn depth_done(dir: &Path) -> bool {
    if !dir.join("depth.png").exists() {
        return false;
    }
    match std::fs::read_to_string(dir.join(DEPTH_MODEL_FILE)) {
        // No marker: the user's own map — keep it.
        Err(_) => true,
        Ok(m) => m.trim() == DEPTH_MODEL,
    }
}

pub fn pending(games: &[Game]) -> usize {
    games.iter().filter(|g| todo(g).any()).count()
}

pub struct Enhancer {
    env: Option<ort::environment::Environment>,
    depth: Option<Session>,
    sr: Option<Session>,
    seg: Option<Session>,
}

/// Model download/load problems: worth retrying next time, unlike a bad image.
const MODEL_ERR: &str = "model: ";

impl Enhancer {
    pub fn new() -> Self {
        Self { env: None, depth: None, sr: None, seg: None }
    }

    /// Upscales and/or depth-maps one game's hero. Errors are reported, not fatal.
    pub fn enhance(&mut self, game: &Game) -> Result<Enhanced, String> {
        let Todo { sr: want_sr, depth: want_depth, segment: want_seg } = todo(game);
        let mut out = Enhanced { id: game.id.clone(), upscaled: false, depth: false, segment: false };
        let dir = art::game_dir(&game.id);
        if want_sr {
            let hero = art::cached(&game.id, ArtKind::Hero).ok_or("no hero")?;
            let img = image::open(&hero).map_err(|e| e.to_string())?;
            let up = self.upscale(&img)?;
            // Keep the original, then write the upscaled one where the dashboard looks.
            let ext = hero.extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_string();
            let _ = std::fs::rename(&hero, dir.join(format!("hero-original.{ext}")));
            save_jpeg(&up, &dir.join("hero.jpg"), 95)?;
            let _ = std::fs::write(dir.join("hero.sr"), b"");
            // A new image needs new maps.
            let _ = std::fs::remove_file(dir.join("depth.png"));
            let _ = std::fs::remove_file(dir.join(DEPTH_MODEL_FILE));
            let _ = std::fs::remove_file(dir.join("segment.png"));
            out.upscaled = true;
        }
        if want_depth || out.upscaled {
            let hero = art::cached(&game.id, ArtKind::Hero).ok_or("no hero")?;
            let img = image::open(&hero).map_err(|e| e.to_string())?;
            let depth = self.depth_map(&img)?;
            depth.save(dir.join("depth.png")).map_err(|e| e.to_string())?;
            // Record which model made the map; a swap regenerates what the old one wrote.
            let _ = std::fs::write(dir.join(DEPTH_MODEL_FILE), DEPTH_MODEL);
            out.depth = true;
        }
        if want_seg || out.upscaled {
            let hero = art::cached(&game.id, ArtKind::Hero).ok_or("no hero")?;
            let img = image::open(&hero).map_err(|e| e.to_string())?;
            let seg = self.segment(&img)?;
            seg.save(dir.join("segment.png")).map_err(|e| e.to_string())?;
            out.segment = true;
        }
        Ok(out)
    }

    /// Semantic regions (see `SEG_GROUPS`), as probabilities. The model wants square input,
    /// so wide art is covered by overlapping square crops whose results are averaged.
    fn segment(&mut self, img: &DynamicImage) -> Result<image::RgbaImage, String> {
        let path = ensure_model(SEG_FILE, SEG_URL, SEG_SHA, None)?;
        let session = Self::session(&mut self.env, &mut self.seg, &path)?;
        let (w, h) = img.dimensions();
        let side = w.min(h).max(1);
        let long = w.max(h);
        let tiles = long.div_ceil(side).max(1);
        // Map resolution: the model's output (1/4 of its input) per crop side.
        let out_side = SEG_IN / 4;
        let (mw, mh) = (
            ((w as f32 / side as f32) * out_side as f32).round().max(1.0) as usize,
            ((h as f32 / side as f32) * out_side as f32).round().max(1.0) as usize,
        );
        let mut acc = vec![[0f32; 4]; mw * mh];
        let mut cnt = vec![0f32; mw * mh];
        let (mean, std) = ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225]);
        let plane = (SEG_IN * SEG_IN) as usize;
        for k in 0..tiles {
            let off = if tiles == 1 { 0 } else { (k as f32 * (long - side) as f32 / (tiles - 1) as f32).round() as u32 };
            let (cx, cy) = if w >= h { (off, 0) } else { (0, off) };
            let crop = img
                .crop_imm(cx, cy, side, side)
                .resize_exact(SEG_IN, SEG_IN, image::imageops::FilterType::Triangle)
                .to_rgb8();
            let mut data = vec![0f32; 3 * plane];
            for (i, p) in crop.pixels().enumerate() {
                for c in 0..3 {
                    data[c * plane + i] = (p[c] as f32 / 255.0 - mean[c]) / std[c];
                }
            }
            let input = Tensor::from_array((vec![1i64, 3, SEG_IN as i64, SEG_IN as i64], data)).map_err(|e| e.to_string())?;
            let outputs = session.run(ort::inputs![input]).map_err(|e| e.to_string())?;
            let (shape, logits) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
            let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
            let (classes, oh, ow) = (dims[1], dims[2], dims[3]);
            let op = oh * ow;
            let ox = (cx as f32 / side as f32 * out_side as f32).round() as usize;
            let oy = (cy as f32 / side as f32 * out_side as f32).round() as usize;
            let mut e = vec![0f32; classes];
            for y in 0..oh {
                for x in 0..ow {
                    let i = y * ow + x;
                    let m = (0..classes).map(|c| logits[c * op + i]).fold(f32::MIN, f32::max);
                    let mut sum = 0.0;
                    for c in 0..classes {
                        e[c] = (logits[c * op + i] - m).exp();
                        sum += e[c];
                    }
                    let (gx, gy) = (ox + x * out_side as usize / ow, oy + y * out_side as usize / oh);
                    if gx >= mw || gy >= mh {
                        continue;
                    }
                    let g = gy * mw + gx;
                    // Crops overlap; trust each one less toward its edges along the long axis,
                    // so tile boundaries don't show up as hard steps.
                    let along = if w >= h { x as f32 / (ow - 1).max(1) as f32 } else { y as f32 / (oh - 1).max(1) as f32 };
                    let wt = if tiles == 1 { 1.0 } else { (along.min(1.0 - along) / 0.2).clamp(0.02, 1.0) };
                    for (ch, group) in SEG_GROUPS.iter().enumerate() {
                        acc[g][ch] += wt * group.iter().map(|&c| e.get(c).copied().unwrap_or(0.0)).sum::<f32>() / sum;
                    }
                    cnt[g] += wt;
                }
            }
        }
        Ok(image::RgbaImage::from_fn(mw as u32, mh as u32, |x, y| {
            let i = y as usize * mw + x as usize;
            let n = cnt[i].max(1e-3);
            image::Rgba(acc[i].map(|v| ((v / n) * 255.0).round().clamp(0.0, 255.0) as u8))
        }))
    }

    fn session<'a>(
        env: &mut Option<ort::environment::Environment>,
        slot: &'a mut Option<Session>,
        path: &Path,
    ) -> Result<&'a mut Session, String> {
        if slot.is_none() {
            let err = |e: &dyn std::fmt::Display| format!("{MODEL_ERR}loading {}: {e}", path.display());
            if env.is_none() {
                *env = Some(ort::init().with_name("gdash").build().map_err(|e| err(&e))?);
            }
            // Leave some cores for the dashboard and whatever else is running.
            let threads = std::thread::available_parallelism().map(|n| n.get() / 2).unwrap_or(4).clamp(1, 6);
            let mut b = Session::builder(env.as_ref().unwrap()).map_err(|e| err(&e))?;
            b = b.with_optimization_level(GraphOptimizationLevel::Level3).map_err(|e| err(&e))?;
            b = b.with_intra_threads(threads).map_err(|e| err(&e))?;
            *slot = Some(b.commit_from_file(path).map_err(|e| err(&e))?);
        }
        Ok(slot.as_mut().unwrap())
    }

    /// Relative depth (near = white), at inference resolution, smoothed edge-aware: flat
    /// inside subjects, sharp across the picture's own edges.
    fn depth_map(&mut self, img: &DynamicImage) -> Result<image::GrayImage, String> {
        let path = ensure_model(DEPTH_FILE, DEPTH_URL, DEPTH_SHA, None)?;
        ensure_companions(&[(DEPTH_DATA_FILE, DEPTH_DATA_URL, DEPTH_DATA_SHA)])?;
        let session = Self::session(&mut self.env, &mut self.depth, &path)?;
        let (w, h) = img.dimensions();
        let scale = DEPTH_SHORT as f32 / w.min(h) as f32;
        let round14 = |v: f32| ((v / 14.0).round().max(1.0) * 14.0) as u32;
        let (iw, ih) = (round14(w as f32 * scale), round14(h as f32 * scale));
        let rgb = img.resize_exact(iw, ih, image::imageops::FilterType::CatmullRom).to_rgb8();
        let (mean, std) = ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225]);
        let plane = (iw * ih) as usize;
        let mut data = vec![0f32; 3 * plane];
        for (i, p) in rgb.pixels().enumerate() {
            for c in 0..3 {
                data[c * plane + i] = (p[c] as f32 / 255.0 - mean[c]) / std[c];
            }
        }
        // The export wraps DA3's multi-view API: [batch, views, 3, h, w]. One view here.
        let input = Tensor::from_array((vec![1i64, 1, 3, ih as i64, iw as i64], data)).map_err(|e| e.to_string())?;
        let outputs = session.run(ort::inputs![input]).map_err(|e| e.to_string())?;
        let (shape, raw) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let dims: Vec<i64> = shape.iter().copied().collect();
        let (oh, ow) = (dims[dims.len() - 2] as usize, dims[dims.len() - 1] as usize);
        // DA3 predicts distance (small = near); the pipeline wants near = white.
        let raw: Vec<f32> = raw[..oh * ow].iter().map(|&v| -v).collect();
        // Robust stretch: the middle 98% of the scene maps to the full range, so one
        // stray extreme pixel can't compress everything else into a sliver of it.
        let mut sorted = raw.clone();
        sorted.sort_by(f32::total_cmp);
        let q = |p: f32| sorted[((sorted.len() - 1) as f32 * p).round() as usize];
        let (lo, hi) = (q(0.01), q(0.99));
        let span = (hi - lo).max(1e-6);
        let d: Vec<f32> = raw.iter().map(|&v| ((v - lo) / span).clamp(0.0, 1.0)).collect();
        // Edge-aware smoothing, guided by the hero's own luminance: depth flattens inside
        // a subject but its boundaries stay crisp, so the depth cards cut clean.
        let guide_img = if (iw, ih) == (ow as u32, oh as u32) {
            rgb
        } else {
            image::imageops::resize(&rgb, ow as u32, oh as u32, image::imageops::FilterType::Triangle)
        };
        let guide: Vec<f32> = guide_img
            .pixels()
            .map(|p| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0)
            .collect();
        let d = joint_bilateral(&d, &guide, ow, oh, 5, 2.2, 0.10);
        Ok(image::GrayImage::from_fn(ow as u32, oh as u32, |x, y| {
            image::Luma([(d[y as usize * ow + x as usize] * 255.0).round() as u8])
        }))
    }

    /// 4x super-resolution in overlapping tiles, then fitted back within 3840x2160.
    fn upscale(&mut self, img: &DynamicImage) -> Result<DynamicImage, String> {
        let path = ensure_model(SR_FILE, SR_URL, SR_SHA, Some("real_esrgan_general_x4v3-onnx-float/"))?;
        let session = Self::session(&mut self.env, &mut self.sr, &path)?;
        let src = img.to_rgb8();
        // Tiny images are padded up to one tile.
        let (sw, sh) = (src.width() as usize, src.height() as usize);
        let (w, h) = (sw.max(SR_TILE), sh.max(SR_TILE));
        let px = |x: usize, y: usize| src.get_pixel(x.min(sw - 1) as u32, y.min(sh - 1) as u32);
        let mut out = RgbImage::new((w * SR_SCALE) as u32, (h * SR_SCALE) as u32);

        let starts = |len: usize| -> Vec<usize> {
            let step = SR_TILE - SR_OVERLAP;
            let mut v: Vec<usize> = (0..).map(|i| i * step).take_while(|&s| s + SR_TILE < len).collect();
            v.push(len - SR_TILE);
            v.dedup();
            v
        };
        let margin = SR_OVERLAP / 2;
        let plane = SR_TILE * SR_TILE;
        for &ty in &starts(h) {
            for &tx in &starts(w) {
                let mut data = vec![0f32; 3 * plane];
                for y in 0..SR_TILE {
                    for x in 0..SR_TILE {
                        let p = px(tx + x, ty + y);
                        for c in 0..3 {
                            data[c * plane + y * SR_TILE + x] = p[c] as f32 / 255.0;
                        }
                    }
                }
                let input = Tensor::from_array((vec![1i64, 3, SR_TILE as i64, SR_TILE as i64], data))
                    .map_err(|e| e.to_string())?;
                let outputs = session.run(ort::inputs![input]).map_err(|e| e.to_string())?;
                let (_, o) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
                // Write the tile's interior; edges that touch the image border are written too.
                let t = SR_TILE * SR_SCALE;
                let x0 = if tx == 0 { 0 } else { margin * SR_SCALE };
                let y0 = if ty == 0 { 0 } else { margin * SR_SCALE };
                let x1 = if tx + SR_TILE >= w { t } else { t - margin * SR_SCALE };
                let y1 = if ty + SR_TILE >= h { t } else { t - margin * SR_SCALE };
                let op = t * t;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let v = |c: usize| (o[c * op + y * t + x].clamp(0.0, 1.0) * 255.0).round() as u8;
                        out.put_pixel((tx * SR_SCALE + x) as u32, (ty * SR_SCALE + y) as u32, image::Rgb([v(0), v(1), v(2)]));
                    }
                }
            }
        }
        // Drop the padding, then fit within 4K.
        let up = DynamicImage::ImageRgb8(out).crop_imm(0, 0, (sw * SR_SCALE) as u32, (sh * SR_SCALE) as u32);
        let (mw, mh) = (3840, 2160);
        let up = if up.width() > mw || up.height() > mh {
            up.resize(mw, mh, image::imageops::FilterType::Lanczos3)
        } else {
            up
        };
        // The model cleans edges well but also smooths painted texture. When the final
        // enlargement is modest, mix some of a plain bicubic upscale back in to keep it.
        let factor = up.width() as f32 / sw as f32;
        let keep = if factor <= 2.5 { 0.35 } else { 0.0 };
        if keep == 0.0 {
            return Ok(up);
        }
        let plain = img.resize_exact(up.width(), up.height(), image::imageops::FilterType::CatmullRom).to_rgb8();
        let mut mixed = up.to_rgb8();
        for (m, p) in mixed.pixels_mut().zip(plain.pixels()) {
            for c in 0..3 {
                m[c] = (m[c] as f32 * (1.0 - keep) + p[c] as f32 * keep).round() as u8;
            }
        }
        Ok(DynamicImage::ImageRgb8(mixed))
    }
}

fn save_jpeg(img: &DynamicImage, path: &Path, quality: u8) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), quality);
    enc.encode_image(&img.to_rgb8()).map_err(|e| e.to_string())
}

/// Edge-aware smoothing for depth maps: a joint bilateral filter. Each value averages its
/// neighbours weighted by distance in the image *and* distance in `guide` (the hero's own
/// luminance, 0..1), so depth flattens inside a subject while refusing to mix across the
/// picture's edges — boundaries survive sharp enough to cut clean depth cards.
fn joint_bilateral(v: &[f32], guide: &[f32], w: usize, h: usize, radius: usize, sigma_s: f32, sigma_r: f32) -> Vec<f32> {
    // The spatial part of the weight is the same for every pixel: one small table
    // instead of an exp() per tap.
    let taps: Vec<(isize, isize, f32)> = {
        let inv = 1.0 / (2.0 * sigma_s * sigma_s);
        let mut t = Vec::with_capacity((2 * radius + 1).pow(2));
        for dy in -(radius as isize)..=radius as isize {
            for dx in -(radius as isize)..=radius as isize {
                t.push((dx, dy, (-(dx * dx + dy * dy) as f32 * inv).exp()));
            }
        }
        t
    };
    let inv_r = 1.0 / (2.0 * sigma_r * sigma_r);
    let at = |buf: &[f32], x: isize, y: isize| {
        buf[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize]
    };
    let mut out = vec![0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let g0 = at(guide, x, y);
            let (mut acc, mut wsum) = (0f32, 0f32);
            for &(dx, dy, ws) in &taps {
                let wr = (-(at(guide, x + dx, y + dy) - g0).powi(2) * inv_r).exp();
                let wt = ws * wr;
                acc += wt * at(v, x + dx, y + dy);
                wsum += wt;
            }
            out[y as usize * w + x as usize] = acc / wsum.max(1e-12);
        }
    }
    out
}

/// Downloads one file and checks it against its pinned SHA-256.
fn fetch_verified(url: &str, file: &str, sha: &str) -> Result<Vec<u8>, String> {
    eprintln!("gdash: downloading {file}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .user_agent("gdash/0.1 (game launcher)")
        .build()
        .into();
    let fail = |e: &dyn std::fmt::Display| format!("{MODEL_ERR}download {file}: {e}");
    let mut resp = agent.get(url).call().map_err(|e| fail(&e))?;
    let bytes = resp.body_mut().with_config().limit(200 * 1024 * 1024).read_to_vec().map_err(|e| fail(&e))?;
    let got = format!("{:x}", Sha256::digest(&bytes));
    if got != sha {
        return Err(fail(&format!("checksum mismatch ({got})")));
    }
    Ok(bytes)
}

/// Downloads (and for zips, extracts) a model once, checking the download's SHA-256.
fn ensure_model(file: &str, url: &str, sha: &str, zip_prefix: Option<&str>) -> Result<PathBuf, String> {
    let dir = models_dir();
    let path = dir.join(file);
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bytes = fetch_verified(url, file, sha)?;
    match zip_prefix {
        None => write_atomic(&path, &bytes)?,
        Some(prefix) => {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
                let Some(name) = entry.name().strip_prefix(prefix).map(str::to_string) else { continue };
                // Only plain file names from the expected folder (the model and its weights).
                if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
                    continue;
                }
                let mut data = Vec::new();
                entry.read_to_end(&mut data).map_err(|e| e.to_string())?;
                write_atomic(&dir.join(&name), &data)?;
            }
            if !path.exists() {
                return Err(format!("{file} missing from archive"));
            }
        }
    }
    Ok(path)
}

/// ONNX external-data companions a graph references by name: downloaded beside the model
/// file and verified the same way. (The file name must stay what the graph references.)
fn ensure_companions(files: &[(&str, &str, &str)]) -> Result<(), String> {
    let dir = models_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (file, url, sha) in files {
        let path = dir.join(file);
        if path.exists() {
            continue;
        }
        let bytes = fetch_verified(url, file, sha)?;
        write_atomic(&path, &bytes)?;
    }
    Ok(())
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// One progress line to the dashboard, which reads this process's stdout over a pipe.
/// If the dashboard exits first, the pipe breaks — that must not kill the pass: the files
/// on disk are the point, the live relay is a nicety. From the first failed write on,
/// this stays quiet and lets the work carry on.
fn say(live: &mut bool, line: impl std::fmt::Display) {
    if !*live {
        return;
    }
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
        *live = false;
    }
}

/// `gdash --enhance-art`: enhances every game that needs it, printing one line per game.
/// The dashboard reads these lines to reload art as it improves — and if it exits before
/// the queue is done, the pass keeps going quietly instead of dying with its pipe.
pub fn run_cli(games: &[Game]) {
    // One enhancer at a time (a second dashboard, or a manual run).
    let lock = models_dir().join("enhance.lock");
    let _ = std::fs::create_dir_all(models_dir());
    let stale = std::fs::metadata(&lock)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(30 * 60));
    if stale {
        let _ = std::fs::remove_file(&lock);
    }
    let mut live = true;
    if std::fs::OpenOptions::new().write(true).create_new(true).open(&lock).is_err() {
        say(&mut live, "total 0");
        return;
    }
    struct Unlock(PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _unlock = Unlock(lock);

    let total = pending(games);
    say(&mut live, format!("total {total}"));
    let mut enhancer = Enhancer::new();
    let mut n = 0;
    for game in games {
        if !todo(game).any() {
            continue;
        }
        n += 1;
        let start = std::time::Instant::now();
        match enhancer.enhance(game) {
            Ok(e) => say(
                &mut live,
                format!(
                    "done {} {} [{n}/{total}] upscaled={} depth={} segment={} {:.1}s",
                    e.id,
                    game.title.replace(' ', "_"),
                    e.upscaled,
                    e.depth,
                    e.segment,
                    start.elapsed().as_secs_f32()
                ),
            ),
            Err(err) if err.starts_with(MODEL_ERR) => {
                // No models (offline?): stop, and try again next time.
                say(&mut live, format!("fail {} {err}", game.id));
                return;
            }
            Err(err) => {
                // Don't retry a broken image forever.
                let _ = std::fs::write(art::game_dir(&game.id).join("depth.none"), err.as_bytes());
                say(&mut live, format!("fail {} {err}", game.id));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_bilateral_flattens_specks_and_keeps_edges() {
        let (w, h) = (40usize, 20);
        // A hard luminance edge at x = 20, with depth stepping across it.
        let guide: Vec<f32> = (0..w * h).map(|i| if i % w < w / 2 { 0.9 } else { 0.1 }).collect();
        let v: Vec<f32> = (0..w * h).map(|i| if i % w < w / 2 { 0.8 } else { 0.2 }).collect();
        let out = joint_bilateral(&v, &guide, w, h, 3, 1.5, 0.08);
        // Pixels beside the edge barely mix with the far side.
        assert!(out[10 * w + 19] > 0.75, "{}", out[10 * w + 19]);
        assert!(out[10 * w + 20] < 0.25, "{}", out[10 * w + 20]);

        // A lone speck on a flat surface is smoothed back into it.
        let mut v = vec![0.5f32; w * h];
        v[5 * w + 5] = 1.0;
        let flat = vec![0.5f32; w * h];
        let out = joint_bilateral(&v, &flat, w, h, 3, 1.5, 0.08);
        assert!(out[5 * w + 5] < 0.6, "{}", out[5 * w + 5]);
    }

    /// `cargo test probe_depth_model -- --ignored --nocapture`: prints the depth model's
    /// I/O contract and runs it on two cached heroes, reporting where the sky band and
    /// the ground band land in the raw output. A development tool for model swaps: the
    /// convention (which end is near) is read from what it prints, not assumed.
    #[test]
    #[ignore]
    fn probe_depth_model() {
        let env = ort::init().with_name("gdash-probe").build().unwrap();
        let path = ensure_model(DEPTH_FILE, DEPTH_URL, DEPTH_SHA, None).unwrap();
        ensure_companions(&[(DEPTH_DATA_FILE, DEPTH_DATA_URL, DEPTH_DATA_SHA)]).unwrap();
        let mut session = Session::builder(&env).unwrap().commit_from_file(&path).unwrap();
        for i in session.inputs() {
            println!("input  {} {:?}", i.name(), i.dtype());
        }
        for o in session.outputs() {
            println!("output {} {:?}", o.name(), o.dtype());
        }
        for id in ["pc-call-of-duty-ghosts", "pc-age-of-mythology-retold"] {
            let Some(hero) = art::cached(id, ArtKind::Hero) else {
                println!("{id}: no hero in the cache");
                continue;
            };
            let img = image::open(&hero).unwrap();
            let (w, h) = img.dimensions();
            let scale = DEPTH_SHORT as f32 / w.min(h) as f32;
            let round14 = |v: f32| ((v / 14.0).round().max(1.0) * 14.0) as u32;
            let (iw, ih) = (round14(w as f32 * scale), round14(h as f32 * scale));
            let rgb = img.resize_exact(iw, ih, image::imageops::FilterType::CatmullRom).to_rgb8();
            let (mean, std) = ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225]);
            let plane = (iw * ih) as usize;
            let mut data = vec![0f32; 3 * plane];
            for (i, p) in rgb.pixels().enumerate() {
                for c in 0..3 {
                    data[c * plane + i] = (p[c] as f32 / 255.0 - mean[c]) / std[c];
                }
            }
            // The export wraps DA3's multi-view API: [batch, views, 3, h, w]; one view here.
            let input = Tensor::from_array((vec![1i64, 1, 3, ih as i64, iw as i64], data)).unwrap();
            let t0 = std::time::Instant::now();
            let outputs = match session.run(ort::inputs![input]) {
                Ok(o) => o,
                Err(e) => {
                    println!("{id}: {iw}x{ih} failed: {e}");
                    continue;
                }
            };
            println!("{id}: {iw}x{ih} ok in {:.2}s", t0.elapsed().as_secs_f32());
            let (shape, depth) = outputs[0].try_extract_tensor::<f32>().unwrap();
            let dims: Vec<i64> = shape.iter().copied().collect();
            let (oh, ow) = (dims[dims.len() - 2] as usize, dims[dims.len() - 1] as usize);
            let v = |x: usize, y: usize| depth[y * ow + x];
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            let (mut top, mut bot) = (0f32, 0f32);
            let band = (oh / 7).max(1);
            for y in 0..oh {
                for x in 0..ow {
                    let d = v(x, y);
                    lo = lo.min(d);
                    hi = hi.max(d);
                    if y < band {
                        top += d;
                    }
                    if y >= oh - band {
                        bot += d;
                    }
                }
            }
            let n = (ow * band) as f32;
            println!(
                "  shape {dims:?}  min {lo:.3} max {hi:.3}  sky-band {:.3}  ground-band {:.3}  center {:.3}",
                top / n,
                bot / n,
                v(ow / 2, oh / 2)
            );
        }
    }

    /// `GDASH_REGEN_IDS=a,b cargo test regen_depth -- --ignored --nocapture`: regenerates
    /// `depth.png` for the listed games with the current pipeline (no SR, no segmentation),
    /// so smoothing and normalization can be iterated on a small set before a full run.
    /// Prints post-processing stats: the far sky band should sit dark (near = white), and
    /// the map should use the whole 0..255 range.
    #[test]
    #[ignore]
    fn regen_depth() {
        let ids = std::env::var("GDASH_REGEN_IDS").expect("GDASH_REGEN_IDS");
        let mut enhancer = Enhancer::new();
        for id in ids.split(',').filter(|s| !s.is_empty()) {
            let Some(hero) = art::cached(id, ArtKind::Hero) else {
                println!("{id}: no hero in the cache");
                continue;
            };
            let img = image::open(&hero).unwrap();
            let t0 = std::time::Instant::now();
            let depth = enhancer.depth_map(&img).unwrap();
            let dir = art::game_dir(id);
            depth.save(dir.join("depth.png")).unwrap();
            let _ = std::fs::write(dir.join(DEPTH_MODEL_FILE), DEPTH_MODEL);
            let (w, h) = (depth.width() as usize, depth.height() as usize);
            let px = depth.as_raw();
            let band = (h / 7).max(1);
            let mean = |ys: std::ops::Range<usize>| {
                let (mut sum, mut n) = (0f64, 0usize);
                for y in ys {
                    for x in 0..w {
                        sum += px[y * w + x] as f64;
                        n += 1;
                    }
                }
                sum / n.max(1) as f64
            };
            let (mut lo, mut hi) = (u8::MAX, u8::MIN);
            for &p in px {
                lo = lo.min(p);
                hi = hi.max(p);
            }
            println!(
                "{id}: {w}x{h} in {:.1}s  range {lo}..{hi}  sky-band {:.0}  ground-band {:.0}",
                t0.elapsed().as_secs_f32(),
                mean(0..band),
                mean(h - band..h)
            );
        }
    }
}
