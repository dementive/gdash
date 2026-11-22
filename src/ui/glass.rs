//! True glass overlays.
//!
//! Bevy has no backdrop blur, so the background is rendered a second time — a
//! quarter-sized copy on its own camera, sharing the very same `BgMaterial` — and the
//! overlay cards sample that copy through their own UI material, blur it further, lift
//! the saturation and tint it. Like CSS `backdrop-filter`, the panels show the picture
//! that is really behind them: racked out of focus, still breathing.
//!
//! The same machinery gives tiles a `ShimmerMaterial`: a soft light sweep for artwork
//! that is still on its way.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, TextureFormat};
use bevy::shader::{Shader, ShaderRef};
use bevy::window::PrimaryWindow;

use super::state::{Dash, Mode};
use super::view::{approach, ease};

pub const GLASS_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("8a2c6f0e-5b41-4d3a-9c72-1e4f8d5a6b03");
pub const SHIMMER_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("c7d94e21-8f36-4b15-a2c9-60d8e3f41b27");
pub const ART_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("b1d3a7c2-9e46-4f85-8a20-5c7e9d4f6a13");

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct GlassMaterial {
    /// x: reveal, 0 = the flat dark card, 1 = full glass.
    #[uniform(0)]
    pub params: Vec4,
    /// The background copy (a quarter of the screen).
    #[texture(1)]
    #[sampler(2)]
    pub backdrop: Handle<Image>,
}

impl UiMaterial for GlassMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(GLASS_SHADER)
    }
}

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct ShimmerMaterial {
    /// x: per-card seed, staggers the sweep.
    #[uniform(0)]
    pub params: Vec4,
}

impl UiMaterial for ShimmerMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHIMMER_SHADER)
    }
}

/// The details sheet's art header (see artcard.wesl): one shared material whose hero
/// texture, focal point and reveal are rewritten as the selection moves.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct ArtCardMaterial {
    /// x: time, y: reveal (0..1), z/w: the focal point (uv) the drift circles.
    #[uniform(0)]
    pub params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub hero: Handle<Image>,
}

impl UiMaterial for ArtCardMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(ART_SHADER)
    }
}

/// The offscreen camera that draws the background copy.
#[derive(Component)]
pub struct GlassCam;
/// The small background quad the copy camera draws (layer 1 is its own little world:
/// the main quad, the particles and the UI all stay off it).
#[derive(Component)]
pub struct BgCopy;
/// An overlay pane: slides up and settles as the overlay opens.
#[derive(Component)]
pub struct GlassCard;

#[derive(Resource)]
pub struct Glass {
    pub mat: Handle<GlassMaterial>,
    /// The details sheet's art header material (updated by view::update_details_sheet).
    pub art: Handle<ArtCardMaterial>,
    target: Handle<Image>,
    size: UVec2,
    /// Smoothed 0..1 with the guide / details sheet.
    reveal: f32,
}

pub fn register(app: &mut App) {
    app.add_plugins((
        UiMaterialPlugin::<GlassMaterial>::default(),
        UiMaterialPlugin::<ShimmerMaterial>::default(),
        UiMaterialPlugin::<ArtCardMaterial>::default(),
    ));
    let glass = Shader::from_wesl(include_str!("shaders/glass.wesl"), "gdash/glass.wesl");
    let shimmer = Shader::from_wesl(include_str!("shaders/shimmer.wesl"), "gdash/shimmer.wesl");
    let art = Shader::from_wesl(include_str!("shaders/artcard.wesl"), "gdash/artcard.wesl");
    let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
    let _ = shaders.insert(GLASS_SHADER.id(), glass);
    let _ = shaders.insert(SHIMMER_SHADER.id(), shimmer);
    let _ = shaders.insert(ART_SHADER.id(), art);
}

/// Quarter of the window — plenty for a blur that will soften it further anyway.
fn target_size(window: Option<&Window>) -> UVec2 {
    let (w, h) = window
        .map(|w| (w.physical_width(), w.physical_height()))
        .unwrap_or((1600, 900));
    UVec2::new((w / 4).max(2), (h / 4).max(2))
}

/// Spawns the copy camera and its background quad, and the glass material the overlay
/// cards will share.
pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    mats: &mut Assets<GlassMaterial>,
    art_mats: &mut Assets<ArtCardMaterial>,
    bg: Handle<super::background::BgMaterial>,
    mesh: Handle<Mesh>,
    window: Option<&Window>,
) -> Glass {
    let size = target_size(window);
    let target =
        images.add(Image::new_target_texture(size.x, size.y, TextureFormat::Bgra8UnormSrgb, None));
    commands.spawn((
        GlassCam,
        Camera2d,
        // Renders before the main camera, so the copy is fresh the same frame.
        Camera { order: -10, ..default() },
        RenderTarget::Image(target.clone().into()),
        Msaa::Off,
        RenderLayers::layer(1),
    ));
    commands.spawn((
        BgCopy,
        Mesh2d(mesh),
        MeshMaterial2d(bg),
        Transform::from_scale(Vec3::new(size.x as f32, size.y as f32, 1.0)),
        RenderLayers::layer(1),
    ));
    Glass {
        mat: mats.add(GlassMaterial { params: Vec4::ZERO, backdrop: target.clone() }),
        art: art_mats.add(ArtCardMaterial { params: Vec4::ZERO, hero: Handle::default() }),
        target,
        size,
        reveal: 0.0,
    }
}

/// Racks the glass in with the overlays, and keeps the background copy shaped like the
/// window (the cover-fit would distort on any other aspect).
pub fn update_glass(
    dash: Res<Dash>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut glass: ResMut<Glass>,
    mut mats: ResMut<Assets<GlassMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cams: Query<&mut RenderTarget, With<GlassCam>>,
    mut copies: Query<&mut Transform, With<BgCopy>>,
) {
    let want = if matches!(dash.mode, Mode::Details | Mode::Guide) { 1.0 } else { 0.0 };
    glass.reveal = approach(glass.reveal, want, 6.0, time.delta_secs().min(0.05));
    if let Some(mut m) = mats.get_mut(&glass.mat) {
        if m.params.x != glass.reveal {
            m.params.x = glass.reveal;
        }
    }

    let want = target_size(windows.single().ok());
    if want != glass.size {
        let target =
            images.add(Image::new_target_texture(want.x, want.y, TextureFormat::Bgra8UnormSrgb, None));
        if let Some(mut m) = mats.get_mut(&glass.mat) {
            m.backdrop = target.clone();
        }
        if let Ok(mut rt) = cams.single_mut() {
            *rt = RenderTarget::Image(target.clone().into());
        }
        if let Ok(mut tf) = copies.single_mut() {
            tf.scale = Vec3::new(want.x as f32, want.y as f32, 1.0);
        }
        glass.target = target;
        glass.size = want;
    }
}

/// The overlay panes rise and settle while the glass racks in.
pub fn animate_cards(
    dash: Res<Dash>,
    time: Res<Time>,
    mut t: Local<f32>,
    mut cards: Query<(&GlassCard, &mut UiTransform)>,
) {
    let want = if matches!(dash.mode, Mode::Details | Mode::Guide) { 1.0 } else { 0.0 };
    *t = approach(*t, want, 7.0, time.delta_secs().min(0.05));
    let e = ease(*t);
    for (_, mut tf) in &mut cards {
        let want = UiTransform {
            translation: Val2::px(0.0, 16.0 * (1.0 - e)),
            scale: Vec2::splat(0.985 + 0.015 * e),
            ..default()
        };
        if *tf != want {
            *tf = want;
        }
    }
}
