//! Camera post effects and the "juice" timeline: the selected game's signature colour,
//! kicks on navigation, the press dip and the launch splash. Bloom is Bevy's own post
//! stack; it only touches the world-space background, so the UI on top stays sharp.

use bevy::post_process::bloom::{Bloom, BloomCompositeMode, BloomPrefilter};
use bevy::input::gamepad::{Gamepad, GamepadAxis};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::input::{Device, NavInput};
use super::state::{Dash, Library, Mode};
use super::textures::Textures;
use crate::library::Game;

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

/// Seconds of quiet browsing before the interface racks itself away and the hero takes
/// the whole screen (see update_ambient and background.wgsl).
pub const AMBIENT_AFTER: f32 = 120.0;

/// Idle ambient only shows while browsing: overlays keep the interface, a launch owns
/// the screen, and the quiet time has to actually pass.
pub fn wants_ambient(mode: Mode, idle: f32, launching: bool) -> bool {
    mode == Mode::Browse && !launching && idle > AMBIENT_AFTER
}

/// A colour pinned by hand in config.toml (`accent = "#rrggbb"` under the game's
/// `[overrides.<id>]`). The `#` is optional. `None` when it isn't a colour, so a typo
/// falls back to what the art says rather than to nothing.
pub fn parse_accent(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    let f = |sh: u32| ((v >> sh) & 0xff) as f32 / 255.0;
    Some(Color::srgb(f(16), f(8), f(0)))
}

/// The game's signature colour: what the user pinned first, then what its art says
/// (extracted from the box art or hero), then whatever the caller decides — usually
/// the blade's colour.
pub fn game_accent(g: &Game, tex: &Textures) -> Option<Color> {
    g.accent.as_deref().and_then(parse_accent).or_else(|| tex.accent(&g.id))
}

#[derive(Resource, Default)]
pub struct Fx {
    /// The selected game's colour (from its art), smoothed. Falls back to the category's.
    pub accent: LinearRgba,
    /// 0..1, racks the interface away after long idle and lets the hero fill the
    /// screen (AMBIENT_AFTER). Rises slowly, falls fast on wake.
    pub ambient: f32,
    /// 0..1, spikes when the selection or category changes and decays quickly.
    pub kick: f32,
    /// Seconds since a game was launched, while the launch splash plays.
    pub launch: Option<f32>,
    /// Where the burst comes from (the selected cover), in screen UV.
    /// Written by the shelf / grid layout each frame.
    pub origin: Vec2,
    /// Seconds since the A button was pressed on a game (the press dip).
    pub press: Option<f32>,
    /// The atmosphere on screen and how much of it is showing (fades between games).
    pub look: GameLook,
    pub look_amount: f32,
    /// 0..1, racks the background out of focus while the guide or details sheet is open.
    pub focus: f32,
    /// Parallax camera offset (drift + right stick / mouse + a push when navigating).
    pub cam: Vec2,
    push: Vec2,
    last: Option<(usize, usize)>,
}

/// Hermite smoothstep between `a` and `b`.
pub fn smoothstep(t: f32, a: f32, b: f32) -> f32 {
    let x = ((t - a) / (b - a)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
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

    /// 0..1, the hero swelling to full-bleed for the launch. Ends before `launch`
    /// expires, so the release never pops.
    pub fn launch_fill(&self) -> f32 {
        match self.launch {
            Some(t) => smoothstep(t, 0.0, 0.85) * (1.0 - smoothstep(t, 1.9, 2.45)),
            None => 0.0,
        }
    }

    /// 0..1, the fade-to-black that carries the logo stamp.
    pub fn launch_flash(&self) -> f32 {
        match self.launch {
            Some(t) => smoothstep(t, 0.55, 0.95) * (1.0 - smoothstep(t, 0.95, 1.70)),
            None => 0.0,
        }
    }

    /// Press dip: a quick push down, then release.
    pub fn press_env(&self) -> f32 {
        match self.press {
            Some(t) if t < 0.06 => t / 0.06,
            Some(t) => (-(t - 0.06) * 9.0).exp(),
            None => 0.0,
        }
    }

    pub fn start_launch(&mut self) {
        self.launch = Some(0.0);
    }

    pub fn start_press(&mut self) {
        self.press = Some(0.0);
    }

    /// The direction of this frame's move (-1, 0 or 1): hero changes read it to enter
    /// from the side you flicked.
    pub fn move_dir(&self) -> f32 {
        self.push.x.signum()
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
    mut cam: Query<&mut Bloom>,
) {
    let dt = time.delta_secs();
    let sel = dash.sel[dash.current];

    let game = dash.selected().and_then(|e| super::view::game_of(&lib, e));
    let target = game.and_then(|g| game_accent(g, &tex)).unwrap_or(dash.blade().color).to_linear();
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

    // Focus pull: overlays take the attention, the picture racks out behind them.
    let want_focus = if matches!(dash.mode, Mode::Details | Mode::Guide) { 1.0 } else { 0.0 };
    if (fx.focus - want_focus).abs() > 1e-4 {
        fx.focus += (want_focus - fx.focus) * (1.0 - (-dt * 6.0).exp());
    }

    // Idle ambient: quiet browsing racks the interface away and the hero spreads to
    // full-bleed (update_ambient drives the chrome, background.wgsl the picture). It
    // rises slowly — nothing to watch — and falls fast, because the touch that wakes
    // the dashboard is swallowed (handle_nav) so it never presses anything.
    let want = wants_ambient(dash.mode, nav.idle, fx.launch.is_some()) as u32 as f32;
    let rate = if want > fx.ambient { 0.55 } else { 8.0 };
    fx.ambient += (want - fx.ambient) * (1.0 - (-rate * dt).exp());
    if (fx.ambient - want).abs() < 1e-3 {
        fx.ambient = want;
    }

    if let Some(t) = fx.launch.as_mut() {
        *t += dt;
        if *t > 2.5 {
            fx.launch = None;
        }
    }
    if let Some(p) = fx.press.as_mut() {
        *p += dt;
        if *p > 0.5 {
            fx.press = None;
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
    // Ambient settles it down further.
    let t = time.elapsed_secs();
    let seed = fx.look.seed * 6.283;
    let drift = Vec2::new((t * 0.11 + seed).sin(), 0.7 * (t * 0.083 + seed * 1.7).sin())
        * (1.0 - 0.5 * fx.ambient);
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
    let Ok(mut bloom) = cam.single_mut() else { return };
    // Only light answers the input — bloom swells a touch while a launch plays out, and
    // eases off while an overlay racks the picture out of focus, so its halos don't
    // smear behind the glass. Nothing bends the picture: no lens pinch, no fringing,
    // no warp.
    let want_bloom = (BLOOM + 0.30 * env) * (1.0 - 0.45 * fx.focus);
    if (bloom.intensity - want_bloom).abs() > 1e-4 {
        bloom.intensity = want_bloom;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(c: Color, r: f32, g: f32, b: f32) -> bool {
        let s = c.to_srgba();
        (s.red - r).abs() < 0.002 && (s.green - g).abs() < 0.002 && (s.blue - b).abs() < 0.002
    }

    #[test]
    fn ambient_wants_quiet_browsing() {
        assert!(wants_ambient(Mode::Browse, AMBIENT_AFTER + 0.1, false));
        // The mark itself is still awake; so is anything short of it.
        assert!(!wants_ambient(Mode::Browse, AMBIENT_AFTER, false));
        assert!(!wants_ambient(Mode::Browse, 1.0, false));
        // Overlays keep the interface, and a launch owns the screen.
        assert!(!wants_ambient(Mode::Guide, f32::MAX, false));
        assert!(!wants_ambient(Mode::Details, f32::MAX, false));
        assert!(!wants_ambient(Mode::Running, f32::MAX, false));
        assert!(!wants_ambient(Mode::Browse, f32::MAX, true));
    }

    #[test]
    fn hand_pinned_accents_parse() {
        assert!(close(parse_accent("#2fd0c0").unwrap(), 0.184, 0.816, 0.753));
        assert!(close(parse_accent("d65bd0").unwrap(), 0.839, 0.357, 0.816));
        // A typo falls back to the art's colour, never to a surprise.
        assert!(parse_accent("#12").is_none());
        assert!(parse_accent("#2fd0cg").is_none());
        assert!(parse_accent("not-a-colour").is_none());
        assert!(parse_accent("").is_none());
    }

    #[test]
    fn pinned_accent_wins_over_the_art() {
        let mut tex = Textures::new();
        let g = Game {
            id: "x".into(),
            title: "X".into(),
            system: crate::library::System::Steam,
            launch: crate::library::Launch::Steam { appid: 1, install_dir: Default::default() },
            steam_appid: Some(1),
            art_name: None,
            last_played: None,
            controller: None,
            accent: Some("#ff0000".into()),
        };
        assert!(close(game_accent(&g, &tex).unwrap(), 1.0, 0.0, 0.0));
        // Nothing pinned and no art loaded: the caller decides.
        let bare = Game { accent: None, ..g };
        assert!(game_accent(&bare, &tex).is_none());
    }

    #[test]
    fn looks_are_stable_and_varied() {
        let c = Color::srgb(0.2, 0.5, 1.0);
        assert_eq!(look_for("steam-570", c), look_for("steam-570", c));
        let styles: std::collections::HashSet<u32> =
            (0..40).map(|i| look_for(&format!("game-{i}"), c).style).collect();
        assert!(styles.len() >= 3, "{styles:?}");
    }

    #[test]
    fn launch_envelopes_never_pop() {
        // The splash must be over (both envelopes back at zero) by the time
        // `launch` expires at 2.5 s, or the release would jump.
        let mut fx = Fx { launch: Some(2.5), ..default() };
        assert!(fx.launch_fill() < 0.001, "fill at expiry: {}", fx.launch_fill());
        assert_eq!(fx.launch_flash(), 0.0);
        // And they start from nothing, so the splash eases in.
        fx.launch = Some(0.0);
        assert_eq!(fx.launch_fill(), 0.0);
        assert_eq!(fx.launch_flash(), 0.0);
        // The flash is a single hump.
        fx.launch = Some(0.95);
        let peak = fx.launch_flash();
        assert!(peak > 0.99, "flash should peak at 0.95 s: {peak}");
        fx.launch = Some(1.4);
        assert!(fx.launch_flash() < peak);
    }

    #[test]
    fn press_dips_fast_and_recovers() {
        let mut fx = Fx { press: Some(0.0), ..default() };
        assert_eq!(fx.press_env(), 0.0);
        fx.press = Some(0.06);
        assert!((fx.press_env() - 1.0).abs() < 1e-6, "full dip at 60 ms");
        fx.press = Some(0.4);
        assert!(fx.press_env() < 0.05, "released well within half a second");
    }
}
