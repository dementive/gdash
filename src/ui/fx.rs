//! Camera post effects and the "juice" timeline: the selected game's signature colour,
//! kicks on navigation, and the launch burst. Bloom, chromatic aberration and lens
//! distortion are Bevy's own post stack; they only touch the world-space background, so
//! the UI on top stays sharp.

use bevy::post_process::bloom::{Bloom, BloomCompositeMode, BloomPrefilter};
use bevy::post_process::effect_stack::{ChromaticAberration, LensDistortion};
use bevy::input::gamepad::{Gamepad, GamepadAxis};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::input::{Device, NavInput};
use super::state::{Dash, Library};
use super::textures::Textures;
use super::theme::{CANVAS_H, SHELF_BOTTOM, SHELF_LEAD, SIDE, TILE_GAP, TILE_SEL_H, TILE_SEL_W, TILE_W};

/// Each game gets its own atmosphere, picked from its id and art colour, so moving through
/// the shelf never feels like the same backdrop with a new picture.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct GameLook {
    /// 0 clear, 1 dust motes, 2 falling snow/ash, 4 bokeh, 5 rising sparks.
    /// (3 was rain; retired.)
    pub style: u32,
    /// 0..1, varies layouts and timing in the shader.
    pub seed: f32,
    /// Light beams from the top-left / top-right corners (strength each).
    pub beams: Vec2,
    /// Beam colour: 0 = the game's colour, 1 = warm white.
    pub beam_tint: f32,
}

pub fn hash(s: &str) -> u64 {
    // FNV-1a, then a mix so neighbouring ids spread out.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^ (h >> 33)
}

pub fn look_for(id: &str, accent: Color) -> GameLook {
    let h = hash(id);
    let f = |shift: u32| ((h >> shift) & 0xffff) as f32 / 65535.0;
    let hue = Hsva::from(accent).hue;
    let sat = Hsva::from(accent).saturation;
    // Warm art leans to sparks/ash/dust, cool art to snow/bokeh. Half the games stay clear,
    // so the atmosphere is an accent rather than constant noise.
    let options: [u32; 4] = if sat < 0.2 {
        [2, 0, 1, 0]
    } else if !(45.0..=330.0).contains(&hue) {
        [5, 0, 1, 0]
    } else if (170.0..=290.0).contains(&hue) {
        [2, 0, 4, 0]
    } else {
        [1, 0, 4, 0]
    };
    let style = options[(h % 4) as usize];
    let beams = match (h >> 8) % 5 {
        0 => Vec2::new(0.9, 0.0),
        1 => Vec2::new(0.0, 0.9),
        _ => Vec2::new(0.55 + 0.45 * f(12), 0.55 + 0.45 * f(28)),
    };
    GameLook {
        style,
        seed: f(4),
        beams,
        beam_tint: f(20),
    }
}

#[derive(Resource, Default)]
pub struct Fx {
    /// The selected game's colour (from its art), smoothed. Falls back to the category's.
    pub accent: LinearRgba,
    /// 0..1, spikes when the selection or category changes and decays quickly.
    pub kick: f32,
    /// Seconds since a game was launched, while the launch burst plays.
    pub launch: Option<f32>,
    /// Where the burst comes from (the selected cover), in screen UV.
    pub origin: Vec2,
    /// The atmosphere on screen and how much of it is showing (fades between games).
    pub look: GameLook,
    pub look_amount: f32,
    /// Parallax camera offset (drift + right stick / mouse + a push when navigating).
    pub cam: Vec2,
    push: Vec2,
    last: Option<(usize, usize)>,
}

impl Fx {
    pub fn accent_color(&self) -> Color {
        Color::LinearRgba(self.accent)
    }

    /// Launch envelope: a sharp attack, then a long tail.
    pub fn launch_env(&self) -> f32 {
        match self.launch {
            Some(t) if t < 0.12 => t / 0.12,
            Some(t) => (-(t - 0.12) * 3.2).exp(),
            None => 0.0,
        }
    }

    pub fn start_launch(&mut self) {
        self.launch = Some(0.0);
    }
}

const BLOOM: f32 = 0.12;

pub fn camera() -> impl Bundle {
    (
        Camera2d,
        // UI anti-aliases itself and the background is one quad: MSAA would only cost memory.
        Msaa::Off,
        Bloom {
            intensity: BLOOM,
            low_frequency_boost: 0.25,
            low_frequency_boost_curvature: 0.9,
            high_pass_frequency: 0.8,
            prefilter: BloomPrefilter { threshold: 0.75, threshold_softness: 0.3 },
            composite_mode: BloomCompositeMode::Additive,
            max_mip_dimension: 512,
            scale: Vec2::ONE,
        },
        ChromaticAberration { intensity: 0.0, ..default() },
        LensDistortion { intensity: 0.0, ..default() },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn update_fx(
    dash: Res<Dash>,
    lib: Res<Library>,
    tex: Res<Textures>,
    time: Res<Time>,
    mut fx: ResMut<Fx>,
    windows: Query<&Window, With<PrimaryWindow>>,
    pads: Query<&Gamepad>,
    nav: Res<NavInput>,
    mut cam: Query<(&mut Bloom, &mut ChromaticAberration, &mut LensDistortion)>,
) {
    let dt = time.delta_secs();
    let sel = dash.sel[dash.current];

    let game = dash.selected().and_then(|e| super::view::game_of(&lib, e));
    let target = game.and_then(|g| tex.accent(&g.id)).unwrap_or(dash.blade().color).to_linear();
    if fx.accent == LinearRgba::NONE {
        fx.accent = target;
    } else {
        let k = 1.0 - (-dt * 5.0).exp();
        fx.accent = fx.accent.mix(&target, k);
    }

    let key = (dash.current, sel);
    match fx.last {
        Some(last) if last != key => {
            let strength = if last.0 != key.0 { 1.0 } else { 0.4 };
            fx.kick = fx.kick.max(strength);
            // The scene swings the way you moved, then settles.
            let dir = if last.0 != key.0 {
                if key.0 > last.0 { 1.0 } else { -1.0 }
            } else if key.1 > last.1 {
                0.6
            } else {
                -0.6
            };
            fx.push.x += dir;
        }
        _ => {}
    }
    fx.last = Some(key);
    fx.kick *= (-dt * 5.0).exp();

    if let Some(t) = fx.launch.as_mut() {
        *t += dt;
        if *t > 2.5 {
            fx.launch = None;
        }
    }
    if fx.launch.is_none() {
        // Track the selected cover so a launch bursts from it.
        if let Ok(w) = windows.single() {
            let canvas_w = w.width() / (w.height() / CANVAS_H).max(0.3);
            let x = SIDE + sel.min(SHELF_LEAD) as f32 * (TILE_W + TILE_GAP) + TILE_SEL_W * 0.5;
            let y = CANVAS_H - SHELF_BOTTOM - TILE_SEL_H * 0.5;
            fx.origin = Vec2::new(x / canvas_w, y / CANVAS_H);
        }
    }

    // Per-game atmosphere: fade the old one out, swap, fade the new one in.
    let look_id = game.map(|g| g.id.clone()).unwrap_or_else(|| format!("blade-{}", dash.current));
    let want = look_for(&look_id, Color::LinearRgba(target));
    if want != fx.look {
        fx.look_amount -= dt * 3.0;
        if fx.look_amount <= 0.0 {
            fx.look_amount = 0.0;
            fx.look = want;
        }
    } else {
        fx.look_amount = (fx.look_amount + dt * 0.8).min(1.0);
    }

    // Parallax camera: a slow drift, plus look-around from the right stick or the mouse.
    let t = time.elapsed_secs();
    let seed = fx.look.seed * 6.283;
    let drift = Vec2::new((t * 0.11 + seed).sin(), 0.7 * (t * 0.083 + seed * 1.7).sin());
    let mut aim = Vec2::ZERO;
    for pad in &pads {
        let v = Vec2::new(
            pad.get(GamepadAxis::RightStickX).unwrap_or(0.0),
            -pad.get(GamepadAxis::RightStickY).unwrap_or(0.0),
        );
        if v.length() > 0.15 {
            aim = v;
        }
    }
    if aim == Vec2::ZERO && nav.device == Device::Keyboard {
        if let Some((w, c)) = windows.single().ok().and_then(|w| w.cursor_position().map(|c| (w, c))) {
            aim = (c / Vec2::new(w.width(), w.height()) - 0.5) * 2.0;
        }
    }
    fx.push *= (-dt * 2.5).exp();
    let goal = drift * 0.55 + aim.clamp_length_max(1.0) * 1.1 + fx.push;
    let k = 1.0 - (-dt * 3.0).exp();
    fx.cam = fx.cam.lerp(goal, k);

    let env = fx.launch_env();
    let kick = fx.kick * fx.kick;
    let Ok((mut bloom, mut ca, mut lens)) = cam.single_mut() else { return };
    let want_bloom = BLOOM + 0.30 * env;
    if (bloom.intensity - want_bloom).abs() > 1e-4 {
        bloom.intensity = want_bloom;
    }
    let want_ca = 0.010 * kick + 0.05 * env;
    if (ca.intensity - want_ca).abs() > 1e-5 {
        ca.intensity = want_ca;
    }
    // Category switches pinch slightly; a launch bulges out toward the viewer.
    let want_lens = -0.06 * kick + 0.45 * env;
    if (lens.intensity - want_lens).abs() > 1e-5 {
        lens.intensity = want_lens;
        lens.scale = 1.0 + 0.35 * want_lens.max(0.0) + 0.06 * (-want_lens).max(0.0);
        lens.center = if env > 0.0 { fx.origin } else { Vec2::splat(0.5) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_are_stable_and_varied() {
        let c = Color::srgb(0.2, 0.5, 1.0);
        assert_eq!(look_for("steam-570", c), look_for("steam-570", c));
        let styles: std::collections::HashSet<u32> =
            (0..40).map(|i| look_for(&format!("game-{i}"), c).style).collect();
        assert!(styles.len() >= 3, "{styles:?}");
    }
}
