//! Streams cached artwork into GPU textures on demand and drops what isn't on screen.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::background::HeroScene;
use super::scene::Emitter;
use crate::art::{self, ArtKind};

type Key = (String, ArtKind);

enum Entry {
    Loading,
    Ready { handle: Handle<Image>, size: UVec2, last_used: f64 },
    Missing,
}

struct Loaded {
    key: Key,
    pixels: Option<(u32, u32, Vec<u8>)>,
    accent: Option<Color>,
    /// For hero art: what the moving background needs.
    scene: Option<SceneData>,
}

struct SceneData {
    w: u32,
    h: u32,
    /// Layered depth (rigid cards), near = white.
    cards: Vec<u8>,
    fxmap: Vec<u8>,
    focal: Vec2,
    emitters: Vec<Emitter>,
}

#[derive(Resource)]
pub struct Textures {
    map: HashMap<Key, Entry>,
    req_tx: Sender<Key>,
    done_rx: Mutex<Receiver<Loaded>>,
    now: f64,
    /// Signature colour per (game, kind), kept after the texture itself is evicted.
    accents: HashMap<Key, Option<Color>>,
    scenes: HashMap<String, HeroScene>,
}

impl Textures {
    pub fn new() -> Self {
        let (req_tx, req_rx) = channel::<Key>();
        let (done_tx, done_rx) = channel::<Loaded>();
        let req_rx = Arc::new(Mutex::new(req_rx));
        for _ in 0..3 {
            let rx = req_rx.clone();
            let tx = done_tx.clone();
            std::thread::spawn(move || {
                loop {
                    let Ok(key) = rx.lock().unwrap().recv() else { break };
                    let pixels = art::cached(&key.0, key.1)
                        .and_then(|p| image::open(p).ok())
                        .map(|img| {
                            let rgba = img.to_rgba8();
                            (rgba.width(), rgba.height(), rgba.into_raw())
                        });
                    let accent = match key.1 {
                        ArtKind::Cover | ArtKind::Hero => pixels.as_ref().and_then(|(_, _, px)| signature_color(px)),
                        _ => None,
                    };
                    let scene = match (key.1, &pixels) {
                        (ArtKind::Hero, Some((w, h, px))) => {
                            let (dw, dh, d) = super::depth::load_override(&key.0)
                                .unwrap_or_else(|| super::depth::estimate(*w, *h, px));
                            let (fx, fy) = super::depth::focal(dw, dh, &d);
                            let seg = super::scene::load_segment(&key.0, dw, dh);
                            let people = super::scene::subjects(seg.as_deref());
                            let cards = super::depth::layered(dw, dh, &d, people.as_deref());
                            let a = super::scene::analyse(*w, *h, px, dw, dh, &cards, seg.as_deref());
                            Some(SceneData { w: dw, h: dh, cards, fxmap: a.fxmap, focal: Vec2::new(fx, fy), emitters: a.emitters })
                        }
                        _ => None,
                    };
                    if tx.send(Loaded { key, pixels, accent, scene }).is_err() {
                        break;
                    }
                }
            });
        }
        Self { map: HashMap::new(), req_tx, done_rx: Mutex::new(done_rx), now: 0.0, accents: HashMap::new(), scenes: HashMap::new() }
    }

    /// Returns the texture if ready, requesting it otherwise.
    pub fn get(&mut self, id: &str, kind: ArtKind) -> Option<(Handle<Image>, UVec2)> {
        let key = (id.to_string(), kind);
        match self.map.get_mut(&key) {
            Some(Entry::Ready { handle, size, last_used }) => {
                *last_used = self.now;
                Some((handle.clone(), *size))
            }
            Some(_) => None,
            None => {
                self.map.insert(key.clone(), Entry::Loading);
                let _ = self.req_tx.send(key);
                None
            }
        }
    }

    /// The game's signature colour (from its box art, else its hero image), once known.
    pub fn accent(&self, id: &str) -> Option<Color> {
        [ArtKind::Cover, ArtKind::Hero]
            .iter()
            .find_map(|k| self.accents.get(&(id.to_string(), *k)).copied().flatten())
    }

    /// Depth cards, effect map, focal point and emitters of a game's hero image (loaded along
    /// with the hero).
    pub fn scene(&self, id: &str) -> Option<HeroScene> {
        self.scenes.get(id).cloned()
    }

    /// True when we know this art doesn't exist (so callers can show a fallback).
    pub fn is_missing(&self, id: &str, kind: ArtKind) -> bool {
        matches!(self.map.get(&(id.to_string(), kind)), Some(Entry::Missing))
    }

    /// Forget misses for a game (new art was just downloaded).
    pub fn invalidate(&mut self, id: &str) {
        self.map.retain(|(gid, _), e| !(gid == id && matches!(e, Entry::Missing)));
    }

    /// Drop everything loaded for a game so improved files on disk are picked up.
    pub fn reload(&mut self, id: &str) {
        self.map.retain(|(gid, _), _| gid != id);
        self.scenes.remove(id);
    }

    pub fn invalidate_all(&mut self) {
        self.map.clear();
    }
}

pub fn pump_textures(mut tex: ResMut<Textures>, mut images: ResMut<Assets<Image>>, time: Res<Time>) {
    tex.now = time.elapsed_secs_f64();
    let loaded: Vec<Loaded> = tex.done_rx.lock().unwrap().try_iter().collect();
    for l in loaded {
        if matches!(l.key.1, ArtKind::Cover | ArtKind::Hero) {
            tex.accents.insert(l.key.clone(), l.accent);
        }
        if let Some(sc) = l.scene {
            let mut upload = |px: Vec<u8>| {
                images.add(Image::new(
                    Extent3d { width: sc.w, height: sc.h, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    px,
                    TextureFormat::Rgba8Unorm,
                    RenderAssetUsages::RENDER_WORLD,
                ))
            };
            let scene = HeroScene {
                depth: upload(sc.cards),
                fxmap: upload(sc.fxmap),
                focal: sc.focal,
                emitters: Arc::new(sc.emitters),
            };
            tex.scenes.insert(l.key.0.clone(), scene);
        }
        let entry = match l.pixels {
            Some((w, h, px)) => {
                let image = Image::new(
                    Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    px,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                );
                Entry::Ready { handle: images.add(image), size: UVec2::new(w, h), last_used: tex.now }
            }
            None => Entry::Missing,
        };
        tex.map.insert(l.key, entry);
    }

    // Evict textures that haven't been drawn for a while (icons are cheap, keep longer).
    let now = tex.now;
    let mut dropped_heroes = Vec::new();
    tex.map.retain(|(id, kind), e| match e {
        Entry::Ready { last_used, .. } => {
            let ttl = if *kind == ArtKind::Icon { 300.0 } else { 20.0 };
            let keep = now - *last_used < ttl;
            if !keep && *kind == ArtKind::Hero {
                dropped_heroes.push(id.clone());
            }
            keep
        }
        _ => true,
    });
    for id in dropped_heroes {
        tex.scenes.remove(&id);
    }
}

/// The most prominent saturated hue in an RGBA8 image, lifted to a bright, usable accent.
/// Returns `None` for mostly grey art (callers fall back to the category colour).
pub fn signature_color(px: &[u8]) -> Option<Color> {
    const BINS: usize = 24;
    let mut weight = [0.0f32; BINS];
    let mut sum = [[0.0f32; 3]; BINS];
    let n = px.len() / 4;
    let step = (n / 6000).max(1);
    let mut total = 0.0;
    for i in (0..n).step_by(step) {
        let [r, g, b] = [px[i * 4], px[i * 4 + 1], px[i * 4 + 2]].map(|c| c as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let chroma = max - min;
        if max < 0.18 || chroma < 0.12 {
            continue;
        }
        let sat = chroma / max;
        let hue = if max == r {
            ((g - b) / chroma).rem_euclid(6.0)
        } else if max == g {
            (b - r) / chroma + 2.0
        } else {
            (r - g) / chroma + 4.0
        } / 6.0;
        let w = sat * sat * max;
        let bin = ((hue * BINS as f32) as usize).min(BINS - 1);
        weight[bin] += w;
        sum[bin][0] += r * w;
        sum[bin][1] += g * w;
        sum[bin][2] += b * w;
        total += w;
    }
    let samples = (n / step).max(1) as f32;
    if total / samples < 0.03 {
        return None;
    }
    let score = |i: usize| weight[i] + 0.5 * (weight[(i + BINS - 1) % BINS] + weight[(i + 1) % BINS]);
    let best = (0..BINS).max_by(|a, b| score(*a).total_cmp(&score(*b)))?;
    let w = weight[best].max(1e-6);
    let avg = Color::srgb(sum[best][0] / w, sum[best][1] / w, sum[best][2] / w);
    let hsv: Hsva = avg.into();
    Some(Color::from(Hsva::new(hsv.hue, hsv.saturation.clamp(0.5, 0.85), 1.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(r: u8, g: u8, b: u8) -> Vec<u8> {
        std::iter::repeat_n([r, g, b, 255], 400).flatten().collect()
    }

    #[test]
    fn picks_dominant_hue() {
        let mut px = solid(200, 30, 30); // red
        px.extend(solid(20, 20, 20)); // dark, ignored
        let c = signature_color(&px).unwrap().to_srgba();
        assert!(c.red > 0.9 && c.green < 0.5 && c.blue < 0.5, "{c:?}");
    }

    #[test]
    fn grey_art_has_no_accent() {
        assert!(signature_color(&solid(128, 128, 130)).is_none());
    }
}
