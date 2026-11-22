//! Full-screen showcase background, drawn in the world (not the UI) so Bevy's HDR post
//! stack — bloom — works on it. The UI is drawn on top afterwards and stays crisp.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};

pub const SHADER: Handle<Shader> = bevy::asset::uuid_handle!("6f1f9a52-3f3e-4c51-9a3e-1b7d2f0c9a11");

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct BgMaterial {
    /// rgb: smoothed accent of the selected game (linear)
    #[uniform(0)]
    pub tint: Vec4,
    /// x: time, y: mix a→b, z: has a, w: has b
    #[uniform(1)]
    pub params: Vec4,
    #[uniform(2)]
    pub sizes: Vec4,
    /// x: zoom a, y: zoom b, zw: screen size (logical px)
    #[uniform(3)]
    pub zoom: Vec4,
    #[texture(4)]
    #[sampler(5)]
    pub hero_a: Handle<Image>,
    #[texture(6)]
    #[sampler(7)]
    pub hero_b: Handle<Image>,
    /// x: launch flash, y: kick (selection change), zw: flash origin (uv)
    #[uniform(8)]
    pub fx: Vec4,
    /// xy: parallax camera, z/w: hero a/b has a depth map
    #[uniform(9)]
    pub look: Vec4,
    /// x: atmosphere style, y: its amount, z: seed, w: beam tint
    #[uniform(10)]
    pub style: Vec4,
    /// x/y: top-left / top-right beam strength
    #[uniform(11)]
    pub beams: Vec4,
    /// x/y: depth dolly of hero a / b; z/w: relight phase of hero a / b
    #[uniform(12)]
    pub breath: Vec4,
    /// Focal point (from the depth map) of hero a (xy) and b (zw)
    #[uniform(13)]
    pub focal: Vec4,
    #[texture(14)]
    #[sampler(15)]
    pub depth_a: Handle<Image>,
    #[texture(16)]
    #[sampler(17)]
    pub depth_b: Handle<Image>,
    /// Parallax camera of hero a (xy) and b (zw)
    #[uniform(18)]
    pub motion: Vec4,
    /// Effect maps (R fire, G lights, B water, A sky) of hero a / b
    #[texture(19)]
    #[sampler(20)]
    pub fxmap_a: Handle<Image>,
    #[texture(21)]
    #[sampler(22)]
    pub fxmap_b: Handle<Image>,
    /// x: 0 showcase layout, 1 full-screen backdrop (All grid); y: launch swell to
    /// full-screen; z: launch white-out.
    #[uniform(23)]
    pub layout: Vec4,
}

impl Material2d for BgMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER)
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Opaque
    }
}

pub fn register(app: &mut App) {
    app.add_plugins(Material2dPlugin::<BgMaterial>::default());
    let shader = Shader::from_wgsl(include_str!("shaders/background.wgsl"), "gdash/background.wgsl");
    let _ = app.world_mut().resource_mut::<Assets<Shader>>().insert(SHADER.id(), shader);
}

/// A hero image on screen and how long it has been showing (drives the zoom).
#[derive(Clone)]
pub struct HeroSlot {
    pub id: String,
    pub image: Handle<Image>,
    pub size: UVec2,
    pub scene: Option<HeroScene>,
    pub age: f32,
}

/// What the scene analysis found in a hero image (see `scene.rs`).
#[derive(Clone)]
pub struct HeroScene {
    /// Layered depth: rigid cards, near = white.
    pub depth: Handle<Image>,
    pub fxmap: Handle<Image>,
    /// Centre of interest (UV).
    pub focal: Vec2,
    pub emitters: Arc<Vec<super::scene::Emitter>>,
}

/// Which game's hero is shown, and the cross-fade from the previous one.
#[derive(Resource, Default)]
pub struct HeroFade {
    /// Previous hero (fading out) and current hero (fading in).
    pub a: Option<HeroSlot>,
    pub b: Option<HeroSlot>,
    pub t: f32,
    /// -1, 0 or 1: the direction the last swap came from (Q/E and shelf moves). The
    /// new hero enters from that side; the old one gives way to it.
    pub dir: f32,
    /// Which hero variant is in slot b (0 = hero, 1 = hero2, 2 = hero3): hero variety
    /// rotates the current game's alternates in place while it stays selected.
    pub variant: u32,
}

/// Seconds the current hero shows before rotating to an alternate of the same game
/// (see view::update_background). Long enough that a browse-through never triggers it.
pub const HERO_SWAP_SECS: f32 = 13.0;
