//! UI sounds synthesised at startup (no audio files to ship).

use std::f32::consts::TAU;
use std::sync::Arc;

use bevy::audio::{AudioPlayer, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;

const RATE: u32 = 44_100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sfx {
    Move,
    Blade,
    Select,
    Back,
    Launch,
    Open,
}

#[derive(Resource)]
pub struct Sounds {
    pub enabled: bool,
    handles: Vec<(Sfx, Handle<AudioSource>)>,
}

impl Sounds {
    pub fn new(assets: &mut Assets<AudioSource>, enabled: bool) -> Self {
        let mk = |samples: Vec<f32>| AudioSource { bytes: Arc::from(wav(&samples)) };
        let handles = vec![
            (Sfx::Move, assets.add(mk(tick()))),
            (Sfx::Blade, assets.add(mk(whoosh()))),
            (Sfx::Select, assets.add(mk(chime(&[880.0, 1318.5], 0.07, 0.28)))),
            (Sfx::Back, assets.add(mk(chime(&[740.0, 494.0], 0.06, 0.20)))),
            (Sfx::Launch, assets.add(mk(chime(&[523.3, 659.3, 784.0, 1046.5], 0.08, 0.9)))),
            (Sfx::Open, assets.add(mk(chime(&[587.3, 880.0], 0.05, 0.3)))),
        ];
        Self { enabled, handles }
    }

    pub fn play(&self, commands: &mut Commands, sfx: Sfx) {
        if !self.enabled {
            return;
        }
        if let Some((_, h)) = self.handles.iter().find(|(s, _)| *s == sfx) {
            commands.spawn((
                AudioPlayer(h.clone()),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.55)),
            ));
        }
    }
}

fn env(t: f32, attack: f32, decay: f32) -> f32 {
    if t < attack { t / attack } else { (-(t - attack) / decay).exp() }
}

fn tick() -> Vec<f32> {
    let n = (RATE as f32 * 0.05) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let s = (TAU * 1650.0 * t).sin() * 0.6 + (TAU * 3300.0 * t).sin() * 0.15;
            s * env(t, 0.002, 0.012) * 0.5
        })
        .collect()
}

fn whoosh() -> Vec<f32> {
    let n = (RATE as f32 * 0.26) as usize;
    let mut seed = 0x1234_5678u32;
    let mut lp = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
            // Sweeping low-pass for the "air" movement.
            let cutoff = 0.02 + 0.25 * (t / 0.26).min(1.0) * (1.0 - t / 0.26).max(0.0) * 2.0;
            lp += (noise - lp) * cutoff;
            let tone = (TAU * (220.0 + 380.0 * t / 0.26) * t).sin() * 0.25;
            (lp * 1.6 + tone) * env(t, 0.04, 0.08) * 0.45
        })
        .collect()
}

fn chime(freqs: &[f32], step: f32, len: f32) -> Vec<f32> {
    let n = (RATE as f32 * (len + step * freqs.len() as f32)) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            freqs
                .iter()
                .enumerate()
                .map(|(k, f)| {
                    let start = k as f32 * step;
                    if t < start {
                        return 0.0;
                    }
                    let tt = t - start;
                    ((TAU * f * tt).sin() + 0.3 * (TAU * f * 2.0 * tt).sin()) * env(tt, 0.004, len * 0.35)
                })
                .sum::<f32>()
                * 0.22
        })
        .collect()
}

/// 16-bit mono PCM WAV.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}
