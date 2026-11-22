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
use super::procedural;

type Key = (String, ArtKind, u32);

/// Variant index within a kind: heroes rotate through `hero` / `hero2` / `hero3`
/// (see art.rs), every other kind only ever has variant 0.
pub const PRIMARY: u32 = 0;

enum Entry {
    Loading,
    Ready { handle: Handle<Image>, size: UVec2, last_used: f64 },
    Missing,
}

/// A generated cover request for the background thread.
struct FbReq {
    id: String,
    accent: Color,
}

enum FbEntry {
    Loading,
    Ready { handle: Handle<Image> },
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
    accents: HashMap<(String, ArtKind), Option<Color>>,
    /// Per hero variant: depth cards, effect map, focal point and emitters.
    scenes: HashMap<(String, u32), HeroScene>,
    /// Hero variant count per game, straight from disk, cached until art changes.
    hero_counts: HashMap<String, usize>,
    /// Generated covers for games with no art (see procedural.rs).
    fallbacks: HashMap<String, FbEntry>,
    fb_tx: Sender<FbReq>,
    fb_done_rx: Mutex<Receiver<(String, Vec<u8>)>>,
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
                    let pixels = art::cached_at(&key.0, key.1, key.2)
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
                            let (dw, dh, d) = super::depth::load_override_at(&key.0, key.2)
                                .unwrap_or_else(|| super::depth::estimate(*w, *h, px));
                            let (fx, fy) = super::depth::focal(dw, dh, &d);
                            let seg = super::scene::load_segment_at(&key.0, key.2, dw, dh);
                            let people = super::scene::subjects(seg.as_deref());
                            let cards = super::depth::layered(dw, dh, &d, people.as_deref());
                            let a = super::scene::analyse(*w, *h, px, dw, dh, &cards, seg.as_deref());
                            Some(SceneData {
                                w: dw,
                                h: dh,
                                cards,
                                fxmap: a.fxmap,
                                focal: Vec2::new(fx, fy),
                                emitters: a.emitters,
                            })
                        }
                        _ => None,
                    };
                    if tx.send(Loaded { key, pixels, accent, scene }).is_err() {
                        break;
                    }
                }
            });
        }
        // One quiet thread for generated covers; requests are rare and small.
        let (fb_tx, fb_rx) = channel::<FbReq>();
        let (fb_done_tx, fb_done_rx) = channel::<(String, Vec<u8>)>();
        std::thread::spawn(move || {
            while let Ok(req) = fb_rx.recv() {
                let px = super::procedural::cover(&req.id, req.accent);
                if fb_done_tx.send((req.id, px)).is_err() {
                    break;
                }
            }
        });
        Self {
            map: HashMap::new(),
            req_tx,
            done_rx: Mutex::new(done_rx),
            now: 0.0,
            accents: HashMap::new(),
            scenes: HashMap::new(),
            hero_counts: HashMap::new(),
            fallbacks: HashMap::new(),
            fb_tx,
            fb_done_rx: Mutex::new(fb_done_rx),
        }
    }

    /// Returns the texture if ready, requesting it otherwise.
    pub fn get(&mut self, id: &str, kind: ArtKind) -> Option<(Handle<Image>, UVec2)> {
        self.get_at(id, kind, PRIMARY)
    }

    /// As `get`, for a specific variant (heroes rotate: 0 = hero, 1 = hero2, 2 = hero3).
    pub fn get_at(&mut self, id: &str, kind: ArtKind, variant: u32) -> Option<(Handle<Image>, UVec2)> {
        let key = (id.to_string(), kind, variant);
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

    /// A hero variant of a game (see art.rs `hero_stem`).
    pub fn get_hero(&mut self, id: &str, variant: u32) -> Option<(Handle<Image>, UVec2)> {
        self.get_at(id, ArtKind::Hero, variant)
    }

    /// How many hero variants a game has on disk (cached; invalidated when its art does).
    pub fn hero_count(&mut self, id: &str) -> usize {
        *self
            .hero_counts
            .entry(id.to_string())
            .or_insert_with(|| art::hero_count(id))
    }

    /// The game's signature colour (from its box art, else its hero image), once known.
    pub fn accent(&self, id: &str) -> Option<Color> {
        [ArtKind::Cover, ArtKind::Hero]
            .iter()
            .find_map(|k| self.accents.get(&(id.to_string(), *k)).copied().flatten())
    }

    /// Depth cards, effect map, focal point and emitters of a hero variant (loaded along
    /// with it).
    pub fn scene(&self, id: &str) -> Option<HeroScene> {
        self.scene_at(id, PRIMARY)
    }

    pub fn scene_at(&self, id: &str, variant: u32) -> Option<HeroScene> {
        self.scenes.get(&(id.to_string(), variant)).cloned()
    }

    /// True when we know this art doesn't exist (so callers can show a fallback).
    pub fn is_missing(&self, id: &str, kind: ArtKind) -> bool {
        matches!(self.map.get(&(id.to_string(), kind, PRIMARY)), Some(Entry::Missing))
    }

    /// The generated cover for a game with no art (see procedural.rs), tinted with
    /// `accent` (usually its system colour). `None` while it is being made; asks for it
    /// on the first call.
    pub fn fallback(&mut self, id: &str, accent: Color) -> Option<Handle<Image>> {
        match self.fallbacks.get(id) {
            Some(FbEntry::Ready { handle }) => Some(handle.clone()),
            Some(FbEntry::Loading) => None,
            None => {
                self.fallbacks.insert(id.to_string(), FbEntry::Loading);
                let _ = self.fb_tx.send(FbReq { id: id.to_string(), accent });
                None
            }
        }
    }

    /// Forget misses for a game (new art was just downloaded).
    pub fn invalidate(&mut self, id: &str) {
        self.map.retain(|(gid, _, _), e| !(gid == id && matches!(e, Entry::Missing)));
        self.fallbacks.remove(id);
        self.hero_counts.remove(id);
    }

    /// Drop everything loaded for a game so improved files on disk are picked up.
    pub fn reload(&mut self, id: &str) {
        self.map.retain(|(gid, _, _), _| gid != id);
        self.scenes.retain(|(gid, _), _| gid != id);
        self.hero_counts.remove(id);
    }

    pub fn invalidate_all(&mut self) {
        self.map.clear();
        self.hero_counts.clear();
    }
}

pub fn pump_textures(mut tex: ResMut<Textures>, mut images: ResMut<Assets<Image>>, time: Res<Time>) {
    tex.now = time.elapsed_secs_f64();

    // Generated covers come back from their thread.
    let done: Vec<(String, Vec<u8>)> = tex.fb_done_rx.lock().unwrap().try_iter().collect();
    for (id, px) in done {
        let image = Image::new(
            Extent3d { width: procedural::W, height: procedural::H, depth_or_array_layers: 1 },
            TextureDimension::D2,
            px,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        let handle = images.add(image);
        tex.fallbacks.insert(id, FbEntry::Ready { handle });
    }

    let loaded: Vec<Loaded> = tex.done_rx.lock().unwrap().try_iter().collect();
    for l in loaded {
        // Only the primary hero tints the game (its accent and atmosphere are its own;
        // alternates are the same game, quietly passing through).
        if matches!(l.key.1, ArtKind::Cover | ArtKind::Hero) && l.key.2 == PRIMARY {
            tex.accents.insert((l.key.0.clone(), l.key.1), l.accent);
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
            tex.scenes.insert((l.key.0.clone(), l.key.2), scene);
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
    tex.map.retain(|(id, kind, _), e| match e {
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
        tex.scenes.retain(|(gid, _), _| gid != &id);
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
