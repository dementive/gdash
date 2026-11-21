//! Showcase Shelf view (see design/SPEC.md): full-bleed hero, category tabs, info block
//! and a shelf of box art.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui::{BackgroundGradient, ColorStop, LinearGradient, RadialGradient, UiScale};
use bevy::window::PrimaryWindow;

use bevy::sprite_render::MeshMaterial2d;

use super::background::{BgMaterial, HeroFade, HeroSlot};
use super::fx::{self, Fx};
use super::glass::{Glass, GlassCard, ShimmerMaterial};
use super::input::{Device, NavInput, PadEvent};
use super::state::{Action, BladeKey, Dash, Entry, GUIDE_ITEMS, Library, Mode};
use super::textures::Textures;
use super::theme::{self, *};
use crate::art::ArtKind;
use crate::library::{ControllerSupport, Game, Launch, System};

// ---------- markers ----------

#[derive(Component)]
pub struct BgNode;
#[derive(Component)]
pub struct Fader;
#[derive(Component)]
pub struct TabsRow;
#[derive(Component)]
pub struct Tab(pub usize);
#[derive(Component)]
pub struct TabText(pub usize);
#[derive(Component)]
pub struct TabUnderline(pub usize);
/// LB / RB key caps (-1 / +1).
#[derive(Component)]
pub struct KeyCap(pub i32);
#[derive(Component)]
pub struct InfoBlock;
/// The "n / total" counter above the shelf.
#[derive(Component)]
pub struct ShelfCount;
#[derive(Component)]
pub struct LogoImg;
#[derive(Component)]
pub struct PlayBtn;
#[derive(Component)]
pub struct DetailsBtn;
#[derive(Component)]
pub struct ShelfLayer;
#[derive(Component)]
pub struct Tile {
    pub idx: usize,
    sel: f32,
    vel: f32,
    appear: f32,
}
/// Glossy-floor reflection under a tile.
#[derive(Component)]
pub struct TileReflection(pub usize);
#[derive(Component)]
pub struct TileReflectionImage(pub usize);

/// A button glyph that flips between its controller and keyboard versions.
#[derive(Component)]
pub struct DeviceSwap {
    order: f32,
    want: Device,
    shown: Device,
    t: f32,
    fresh: bool,
}
#[derive(Component)]
pub struct ForDevice(Device);
#[derive(Component)]
pub struct TileImage(pub usize);
#[derive(Component)]
pub struct TileFallback(pub usize);
/// A light sweep while this tile's artwork is still arriving.
#[derive(Component)]
pub struct TileLoading(pub usize);
#[derive(Component)]
pub struct TileDim(pub usize);
#[derive(Component)]
pub struct Prompts;
#[derive(Component)]
pub struct GuideOverlay;
#[derive(Component)]
pub struct GuideRow(pub usize);
#[derive(Component)]
pub struct DetailsSheet;
#[derive(Component)]
pub struct Toast;
#[derive(Component)]
pub struct RunningOverlay;
/// The white-out of the launch splash; covers the chrome while the stamp shows.
#[derive(Component)]
pub struct LaunchFlash;
/// The game's logo (or name) stamped over the white-out.
#[derive(Component)]
pub struct LaunchStamp;
#[derive(Component)]
pub struct StampLogo;
#[derive(Component)]
pub struct StampText;

/// Text labels updated by content.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Clock,
    Status,
    LogoText,
    Meta,
    PlayLabel,
    Count,
    Toast,
    Running,
    DetTitle,
    DetBody,
}

/// Shelf scroll position.
#[derive(Resource, Default)]
pub struct ShelfAnim {
    scroll: f32,
}

/// Info block slide/fade, restarted whenever the selection changes.
#[derive(Resource, Default)]
pub struct InfoAnim {
    key: (usize, usize, u64),
    t: f32,
}

// ---------- helpers ----------

fn grad(angle: f32, a: Color, b: Color) -> BackgroundGradient {
    BackgroundGradient::from(LinearGradient {
        angle,
        stops: vec![ColorStop::new(a, percent(0)), ColorStop::new(b, percent(100))],
        ..default()
    })
}

fn glossy(c: (Color, Color)) -> BackgroundGradient {
    BackgroundGradient::from(RadialGradient {
        position: UiPosition::anchor(Vec2::new(-0.2, -0.25)),
        shape: bevy::ui::RadialGradientShape::FarthestCorner,
        stops: vec![ColorStop::new(c.0, percent(0)), ColorStop::new(c.1, percent(100))],
        ..default()
    })
}

pub(super) fn shadow(color: Color, y: f32, blur: f32, spread: f32) -> ShadowStyle {
    ShadowStyle { color, x_offset: px(0), y_offset: px(y), spread_radius: px(spread), blur_radius: px(blur) }
}

pub(super) fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub(super) fn approach(cur: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let v = cur + (target - cur) * (1.0 - (-rate * dt).exp());
    if (target - v).abs() < 0.002 * (1.0 + target.abs()) { target } else { v }
}

pub(super) fn set_px(node: &mut Mut<Node>, field: impl Fn(&mut Node) -> &mut Val, v: f32) {
    let cur = *field(node.bypass_change_detection());
    if cur != px(v) {
        *field(node) = px(v);
    }
}

pub(super) fn set_display(node: &mut Mut<Node>, show: bool) {
    let d = if show { Display::Flex } else { Display::None };
    if node.display != d {
        node.display = d;
    }
}

pub(super) fn set_text(text: &mut Mut<Text>, s: &str) {
    if text.0 != s {
        text.0 = s.to_string();
    }
}

pub(super) fn set_color<T: PartialEq>(slot: &mut Mut<T>, v: T) {
    if **slot != v {
        **slot = v;
    }
}

pub fn system_color(sys: System) -> Color {
    match sys {
        System::Steam => theme::STEAM,
        System::Pc => theme::PC,
        System::Xbox => theme::XBOX,
        System::Xbox360 => theme::XBOX360,
        System::Wishlist => theme::WISHLIST,
    }
}

pub(super) fn game_of<'a>(lib: &'a Library, e: &Entry) -> Option<&'a Game> {
    match e.action {
        Action::Launch(i) => lib.games.get(i),
        Action::OpenWishlist(i) => lib.wishlist.get(i),
        _ => None,
    }
}

/// The meta line under a game title: system + last played, or where a wishlist item came from.
pub(super) fn game_meta(g: &Game) -> String {
    if let Launch::Url { url } = &g.launch {
        return format!("Not installed  •  {}", crate::library::wishlist::source_host(url));
    }
    let played = g.last_played.map(|t| format!("Played {}", super::state::ago(t))).unwrap_or("Not played yet".into());
    format!("{}  •  {}", g.system.label(), played)
}

/// A round face-button glyph like the ones on a controller.
fn face_button(p: &mut ChildSpawnerCommands, glyph: &str, colors: (Color, Color), size: f32) {
    p.spawn((
        Node {
            width: px(size),
            height: px(size),
            flex_shrink: 0.0,
            border_radius: BorderRadius::MAX,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        glossy(colors),
        BoxShadow(vec![shadow(Color::srgba(0.0, 0.0, 0.0, 0.35), 2.0, 6.0, 0.0)]),
        Pickable::IGNORE,
    ))
    .with_children(|b| {
        b.spawn((Text::new(glyph), theme::font(size * 0.47, FontWeight::BOLD), TextColor(WHITE), Pickable::IGNORE));
    });
}

/// One button glyph, drawn like the physical control.
#[derive(Clone, Copy)]
enum Glyph {
    Face(&'static str, (Color, Color)),
    Menu,
    Bumper(&'static str),
    Key(&'static str),
}

const MENU: (Color, Color) = (Color::srgb(0.5, 0.55, 0.6), Color::srgb(0.25, 0.28, 0.32));

fn glyph(p: &mut ChildSpawnerCommands, g: Glyph, size: f32) {
    match g {
        Glyph::Face(label, colors) => face_button(p, label, colors, size),
        Glyph::Menu => {
            p.spawn((
                Node {
                    width: px(size),
                    height: px(size),
                    border_radius: BorderRadius::MAX,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                glossy(MENU),
                BoxShadow(vec![shadow(Color::srgba(0.0, 0.0, 0.0, 0.35), 2.0, 6.0, 0.0)]),
                Pickable::IGNORE,
            ))
            .with_children(|b| {
                b.spawn((Text::new("\u{E700}"), theme::icon_font(size * 0.5), TextColor(WHITE), Pickable::IGNORE));
            });
        }
        Glyph::Bumper(label) => {
            p.spawn((
                Node {
                    height: px(size * 0.86),
                    min_width: px(size * 1.3),
                    padding: UiRect::horizontal(px(size * 0.28)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius {
                        top_left: px(size * 0.45).into(),
                        top_right: px(size * 0.45).into(),
                        bottom_left: px(size * 0.16).into(),
                        bottom_right: px(size * 0.16).into(),
                    },
                    ..default()
                },
                grad(LinearGradient::TO_BOTTOM, Color::srgb(0.40, 0.43, 0.48), Color::srgb(0.14, 0.15, 0.18)),
                BoxShadow(vec![shadow(Color::srgba(0.0, 0.0, 0.0, 0.35), 2.0, 5.0, 0.0)]),
                Pickable::IGNORE,
            ))
            .with_children(|b| {
                b.spawn((Text::new(label), theme::font(size * 0.42, FontWeight::BOLD), TextColor(WHITE), Pickable::IGNORE));
            });
        }
        Glyph::Key(label) => {
            let (top, bottom, edge, text) =
                (Color::srgb(0.27, 0.29, 0.34), Color::srgb(0.13, 0.14, 0.17), Color::srgb(0.04, 0.04, 0.06), WHITE);
            p.spawn((
                Node {
                    height: px(size),
                    min_width: px(size),
                    padding: UiRect::horizontal(px(size * 0.3)),
                    border: UiRect::bottom(px(3)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(size * 0.22)),
                    ..default()
                },
                grad(LinearGradient::TO_BOTTOM, top, bottom),
                BorderColor::all(edge),
                BoxShadow(vec![shadow(Color::srgba(0.0, 0.0, 0.0, 0.35), 2.0, 5.0, 0.0)]),
                Pickable::IGNORE,
            ))
            .with_children(|b| {
                b.spawn((Text::new(label), theme::font(size * 0.42, FontWeight::BOLD), TextColor(text), Pickable::IGNORE));
            });
        }
    }
}

/// A glyph that shows the controller or the keyboard version, flipping when the device
/// changes. `order` staggers the flips so a row of prompts cascades.
fn swap_glyph(p: &mut ChildSpawnerCommands, pad: &[Glyph], key: &[Glyph], size: f32, order: f32) {
    p.spawn((
        DeviceSwap { order, want: Device::Keyboard, shown: Device::Keyboard, t: 1.0, fresh: true },
        Node { align_items: AlignItems::Center, ..default() },
        UiTransform::default(),
        Pickable::IGNORE,
    ))
    .with_children(|s| {
        for (dev, glyphs) in [(Device::Gamepad, pad), (Device::Keyboard, key)] {
            s.spawn((
                ForDevice(dev),
                Node {
                    column_gap: px(5),
                    align_items: AlignItems::Center,
                    display: if dev == Device::Keyboard { Display::Flex } else { Display::None },
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|v| {
                for g in glyphs {
                    glyph(v, *g, size);
                }
            });
        }
    });
}

/// A clickable LB / RB (Q / E) cap in the top bar.
fn key_cap(p: &mut ChildSpawnerCommands, pad: &'static str, key: &'static str, dir: i32) {
    p.spawn((KeyCap(dir), Hovered::default(), Node { padding: UiRect::all(px(2)), ..default() })).with_children(|k| {
        swap_glyph(k, &[Glyph::Bumper(pad)], &[Glyph::Key(key)], 26.0, if dir < 0 { 0.0 } else { 1.0 })
    });
}

/// "glyph  Label" pairs for prompt rows. A `slot` makes the label follow the selection.
fn prompt(
    p: &mut ChildSpawnerCommands,
    pad: &[Glyph],
    key: &[Glyph],
    label: &str,
    order: f32,
    slot: Option<Slot>,
) {
    p.spawn((Node { column_gap: px(9), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE))
        .with_children(|it| {
            swap_glyph(it, pad, key, 24.0, order);
            let mut text = it.spawn((
                Text::new(label),
                theme::font(15.0, FontWeight::NORMAL),
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.72)),
                Pickable::IGNORE,
            ));
            if let Some(slot) = slot {
                text.insert(slot);
            }
        });
}

fn orb(p: &mut ChildSpawnerCommands, size: f32) {
    p.spawn((
        Node { width: px(size), height: px(size), flex_shrink: 0.0, border_radius: BorderRadius::MAX, ..default() },
        BackgroundGradient::from(RadialGradient {
            position: UiPosition::anchor(Vec2::new(-0.2, -0.25)),
        shape: bevy::ui::RadialGradientShape::FarthestCorner,
            stops: vec![
                ColorStop::new(Color::srgb(0.88, 1.0, 0.72), percent(0)),
                ColorStop::new(Color::srgb(0.47, 0.86, 0.16), percent(40)),
                ColorStop::new(Color::srgb(0.16, 0.46, 0.04), percent(100)),
            ],
            ..default()
        }),
        BoxShadow(vec![shadow(Color::srgba(0.47, 0.9, 0.24, 0.45), 0.0, 12.0, 0.0)]),
        Pickable::IGNORE,
    ));
}

// ---------- spawning ----------

pub fn spawn_static(
    mut commands: Commands,
    mut mats: ResMut<Assets<BgMaterial>>,
    mut pmats: ResMut<Assets<super::particles::ParticleMaterial>>,
    mut glass_mats: ResMut<Assets<super::glass::GlassMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    // A second camera exists for the glass overlays, so say which one owns the UI.
    commands.spawn((super::fx::camera(), IsDefaultUiCamera));
    super::particles::spawn(&mut commands, &mut meshes, &mut pmats);

    // One quad, two draws: full size on the main camera, and a quarter-size copy for
    // the glass overlays to sample — the same material, so they never drift apart.
    let mesh = meshes.add(Rectangle::new(1.0, 1.0));
    let bg = mats.add(BgMaterial {
        tint: Vec4::ZERO,
        params: Vec4::ZERO,
        sizes: Vec4::ONE,
        zoom: Vec4::ONE,
        hero_a: Handle::default(),
        hero_b: Handle::default(),
        fx: Vec4::ZERO,
        look: Vec4::ZERO,
        style: Vec4::ZERO,
        beams: Vec4::ZERO,
        breath: Vec4::ZERO,
        focal: Vec4::splat(0.5),
        depth_a: Handle::default(),
        depth_b: Handle::default(),
        motion: Vec4::ZERO,
        fxmap_a: Handle::default(),
        fxmap_b: Handle::default(),
        layout: Vec4::ZERO,
    });
    commands.spawn((
        BgNode,
        Mesh2d(mesh.clone()),
        MeshMaterial2d(bg.clone()),
        Transform::from_scale(Vec3::new(1600.0, 900.0, 1.0)),
    ));
    let glass = super::glass::spawn(
        &mut commands,
        &mut images,
        &mut glass_mats,
        bg,
        mesh,
        windows.single().ok(),
    );

    spawn_top_bar(&mut commands);
    spawn_info(&mut commands);

    commands.spawn((
        Slot::Count,
        ShelfCount,
        Text::new(""),
        theme::numeric_font(15.0, FontWeight::NORMAL),
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
        Node {
            position_type: PositionType::Absolute,
            right: px(SIDE),
            bottom: px(SHELF_BOTTOM + TILE_SEL_H + 18.0),
            ..default()
        },
        GlobalZIndex(5),
        Pickable::IGNORE,
    ));

    commands.spawn((
        ShelfLayer,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            bottom: px(SHELF_BOTTOM),
            height: px(TILE_SEL_H),
            ..default()
        },
        GlobalZIndex(6),
        Pickable::IGNORE,
    ));

    // (The old "hero light on the floor" strip is gone: it read as a grey bar laid over the
    // artwork. The picture now runs clean to the bottom edge in every view.)

    commands
        .spawn((
            Prompts,
            Node {
                position_type: PositionType::Absolute,
                left: px(SIDE),
                bottom: px(20),
                column_gap: px(26),
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(20),
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            prompt(row, &[Glyph::Face("A", BTN_A)], &[Glyph::Key("Enter")], "Play", 4.0, Some(Slot::PlayLabel));
            prompt(row, &[Glyph::Face("B", BTN_B)], &[Glyph::Key("Bksp")], "Back", 5.0, None);
            prompt(row, &[Glyph::Face("Y", BTN_Y)], &[Glyph::Key("I")], "Details", 6.0, None);
            prompt(row, &[Glyph::Menu], &[Glyph::Key("Esc")], "Guide", 7.0, None);
            prompt(row, &[Glyph::Bumper("LB"), Glyph::Bumper("RB")], &[Glyph::Key("Q"), Glyph::Key("E")], "Category", 8.0, None);
            prompt(row, &[Glyph::Bumper("LT"), Glyph::Bumper("RT")], &[Glyph::Key("PgUp"), Glyph::Key("PgDn")], "Jump", 9.0, None);
        });

    super::grid::spawn(&mut commands);
    spawn_overlays(&mut commands, &glass);
    commands.insert_resource(glass);

    commands.spawn((
        Fader,
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::BLACK),
        GlobalZIndex(200),
        Pickable::IGNORE,
    ));
}

fn spawn_top_bar(commands: &mut Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(SIDE),
                right: px(SIDE),
                top: px(30),
                height: px(48),
                column_gap: px(26),
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(20),
            Pickable::IGNORE,
        ))
        .with_children(|bar| {
            orb(bar, 26.0);
            key_cap(bar, "LB", "Q", -1);
            bar.spawn((TabsRow, Node { column_gap: px(34), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE));
            key_cap(bar, "RB", "E", 1);
            bar.spawn((Node { flex_grow: 1.0, ..default() }, Pickable::IGNORE));
            bar.spawn((
                Slot::Status,
                Text::new(""),
                theme::numeric_font(15.0, FontWeight::NORMAL),
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.6)),
                Pickable::IGNORE,
            ));
            bar.spawn((Slot::Clock, Text::new(""), theme::numeric_font(20.0, FontWeight::LIGHT), TextColor(WHITE), Pickable::IGNORE));
        });
}

fn spawn_info(commands: &mut Commands) {
    commands
        .spawn((
            InfoBlock,
            Node {
                position_type: PositionType::Absolute,
                left: px(SIDE),
                bottom: px(SHELF_BOTTOM + TILE_SEL_H + 46.0),
                width: px(720),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                ..default()
            },
            UiTransform::default(),
            GlobalZIndex(5),
            Pickable::IGNORE,
        ))
        .with_children(|info| {
            info.spawn((LogoImg, ImageNode::default(), Node { display: Display::None, ..default() }, Pickable::IGNORE));
            info.spawn((
                Slot::LogoText,
                Text::new(""),
                theme::font(58.0, FontWeight::SEMIBOLD),
                // Display sizes read tighter in Segoe UI's default tracking.
                theme::tracking(-0.6),
                TextColor(WHITE),
                TextShadow { offset: Vec2::new(0.0, 4.0), color: Color::srgba(0.0, 0.0, 0.0, 0.75) },
                Node { max_width: px(720), ..default() },
                Pickable::IGNORE,
            ));
            info.spawn((
                Slot::Meta,
                Text::new(""),
                theme::font(18.0, FontWeight::NORMAL),
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
                TextShadow { offset: Vec2::new(0.0, 2.0), color: Color::srgba(0.0, 0.0, 0.0, 0.6) },
                Node { margin: UiRect::top(px(20)), max_width: px(640), ..default() },
                Pickable::IGNORE,
            ));
            info.spawn((Node { column_gap: px(14), margin: UiRect::top(px(26)), ..default() }, Pickable::IGNORE))
                .with_children(|row| {
                    row.spawn((
                        PlayBtn,
                        Hovered::default(),
                        Node {
                            height: px(52),
                            padding: UiRect { left: px(11), right: px(28), ..default() },
                            column_gap: px(12),
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::MAX,
                            border: UiRect::all(px(2)),
                            ..default()
                        },
                        UiTransform::default(),
                        BackgroundColor(Color::srgba(0.07, 0.08, 0.11, 0.92)),
                        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.2)),
                        BoxShadow(vec![shadow(Color::NONE, 0.0, 16.0, 0.0)]),
                    ))
                    .with_children(|b| {
                        swap_glyph(b, &[Glyph::Face("A", BTN_A)], &[Glyph::Key("Enter")], 30.0, 2.0);
                        b.spawn((
                            Slot::PlayLabel,
                            Text::new("Play"),
                            theme::font(19.0, FontWeight::SEMIBOLD),
                            TextColor(WHITE),
                            Pickable::IGNORE,
                        ));
                    });
                    row.spawn((
                        DetailsBtn,
                        Hovered::default(),
                        Node {
                            height: px(52),
                            padding: UiRect { left: px(11), right: px(28), ..default() },
                            column_gap: px(12),
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::MAX,
                            border: UiRect::all(px(1)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.72)),
                        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.12)),
                    ))
                    .with_children(|b| {
                        swap_glyph(b, &[Glyph::Face("Y", BTN_Y)], &[Glyph::Key("I")], 30.0, 3.0);
                        b.spawn((
                            Text::new("Details"),
                            theme::font(19.0, FontWeight::SEMIBOLD),
                            TextColor(WHITE),
                            Pickable::IGNORE,
                        ));
                    });
                });
        });
}

fn glass_card(width: f32, glass: &Glass) -> impl Bundle {
    (
        GlassCard,
        Node {
            width: px(width),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(30)),
            row_gap: px(6),
            border_radius: BorderRadius::all(px(18)),
            border: UiRect::all(px(1)),
            ..default()
        },
        // The blurred, saturated backdrop copy shines through (see glass.rs).
        MaterialNode(glass.mat.clone()),
        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.14)),
        BoxShadow(vec![shadow(Color::srgba(0.0, 0.0, 0.0, 0.55), 30.0, 80.0, 0.0)]),
    )
}

fn spawn_overlays(commands: &mut Commands, glass: &Glass) {
    // Details sheet (right side).
    commands
        .spawn((
            DetailsSheet,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::FlexEnd,
                align_items: AlignItems::Center,
                padding: UiRect::right(px(SIDE)),
                display: Display::None,
                ..default()
            },
            // The pane itself is glass now, so the scrim only has to guide the eye.
            grad(LinearGradient::TO_RIGHT, Color::srgba(0.0, 0.0, 0.0, 0.22), Color::srgba(0.0, 0.0, 0.0, 0.72)),
            GlobalZIndex(40),
        ))
        .with_children(|o| {
            o.spawn(glass_card(560.0, glass)).with_children(|c| {
                c.spawn((Slot::DetTitle, Text::new(""), theme::font(30.0, FontWeight::SEMIBOLD), TextColor(WHITE)));
                c.spawn((
                    Slot::DetBody,
                    Text::new(""),
                    theme::font(17.0, FontWeight::NORMAL),
                    TextColor(Color::srgba(1.0, 1.0, 1.0, 0.82)),
                    Node { margin: UiRect::top(px(14)), ..default() },
                ));
                c.spawn(Node { column_gap: px(22), margin: UiRect::top(px(22)), ..default() }).with_children(|r| {
                    prompt(r, &[Glyph::Face("A", BTN_A)], &[Glyph::Key("Enter")], "Play", 0.0, Some(Slot::PlayLabel));
                    prompt(r, &[Glyph::Face("B", BTN_B)], &[Glyph::Key("Bksp")], "Close", 1.0, None);
                });
            });
        });

    // Guide.
    commands
        .spawn((
            GuideOverlay,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.60)),
            GlobalZIndex(50),
        ))
        .with_children(|o| {
            o.spawn(glass_card(500.0, glass)).with_children(|p| {
                p.spawn(Node { column_gap: px(14), align_items: AlignItems::Center, margin: UiRect::bottom(px(14)), ..default() })
                    .with_children(|h| {
                        orb(h, 34.0);
                        h.spawn((Text::new("Guide"), theme::font(32.0, FontWeight::LIGHT), TextColor(WHITE)));
                    });
                let items = std::iter::once("Resume").chain(GUIDE_ITEMS.iter().map(|(t, _)| *t));
                for (i, label) in items.enumerate() {
                    p.spawn((
                        GuideRow(i),
                        Hovered::default(),
                        Node {
                            height: px(50),
                            padding: UiRect::horizontal(px(18)),
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(px(10)),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                    ))
                    .with_children(|r| {
                        r.spawn((Text::new(label), theme::font(20.0, FontWeight::SEMIBOLD), TextColor(WHITE), Pickable::IGNORE));
                    });
                }
                p.spawn((Node { column_gap: px(22), margin: UiRect::top(px(16)), ..default() }, Pickable::IGNORE))
                    .with_children(|r| {
                        prompt(r, &[Glyph::Face("A", BTN_A)], &[Glyph::Key("Enter")], "Select", 0.0, None);
                        prompt(r, &[Glyph::Face("B", BTN_B)], &[Glyph::Key("Esc")], "Close", 1.0, None);
                    });
            });
        });

    // Toast.
    commands
        .spawn((
            Toast,
            Node {
                position_type: PositionType::Absolute,
                top: px(96),
                width: percent(100),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            GlobalZIndex(60),
            Pickable::IGNORE,
        ))
        .with_children(|t| {
            t.spawn((
                Node { padding: UiRect::axes(px(26), px(11)), border_radius: BorderRadius::MAX, ..default() },
                BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.9)),
                BoxShadow(vec![shadow(Color::srgba(0.0, 0.0, 0.0, 0.5), 8.0, 24.0, 0.0)]),
                Pickable::IGNORE,
            ))
            .with_children(|p| {
                p.spawn((Slot::Toast, Text::new(""), theme::font(18.0, FontWeight::SEMIBOLD), TextColor(WHITE), Pickable::IGNORE));
            });
        });

    // Now playing.
    commands
        .spawn((
            RunningOverlay,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.93)),
            GlobalZIndex(55),
        ))
        .with_children(|o| {
            o.spawn((
                Slot::Running,
                Text::new(""),
                theme::font(28.0, FontWeight::LIGHT),
                TextColor(WHITE),
                TextLayout::justify(Justify::Center),
            ));
        });

    // The launch splash's white-out, over every piece of chrome.
    commands.spawn((
        LaunchFlash,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::NONE),
        GlobalZIndex(150),
        Pickable::IGNORE,
    ));

    // The game's mark, stamped into the white-out (see update_launch_splash).
    commands
        .spawn((
            LaunchStamp,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            UiTransform::default(),
            GlobalZIndex(160),
            Pickable::IGNORE,
        ))
        .with_children(|s| {
            s.spawn((
                StampLogo,
                ImageNode::default(),
                Node { display: Display::None, ..default() },
                Pickable::IGNORE,
            ));
            s.spawn((
                StampText,
                Text::new(""),
                theme::font(64.0, FontWeight::SEMIBOLD),
                // Display sizes read tighter in Segoe UI's default tracking.
                theme::tracking(-0.8),
                TextColor(WHITE),
                TextShadow { offset: Vec2::new(0.0, 6.0), color: Color::srgba(0.0, 0.0, 0.0, 0.8) },
                // Long titles wrap to a centred second line instead of running off-screen.
                TextLayout::justify(Justify::Center),
                Node { display: Display::None, max_width: px(1200), ..default() },
                Pickable::IGNORE,
            ));
        });
}

// ---------- scale & tabs ----------

pub fn update_scale(windows: Query<&Window, With<PrimaryWindow>>, mut scale: ResMut<UiScale>) {
    if let Ok(w) = windows.single() {
        let s = (w.height() / CANVAS_H).max(0.3);
        if (scale.0 - s).abs() > 1e-4 {
            scale.0 = s;
        }
    }
}

pub fn sync_tabs(
    mut commands: Commands,
    dash: Res<Dash>,
    row: Query<Entity, With<TabsRow>>,
    mut last: Local<Option<u64>>,
) {
    if *last == Some(dash.layout_gen) {
        return;
    }
    let Ok(row) = row.single() else { return };
    *last = Some(dash.layout_gen);
    commands.entity(row).despawn_children();
    commands.entity(row).with_children(|r| {
        for (i, b) in dash.blades.iter().enumerate() {
            r.spawn((Tab(i), Hovered::default(), Node { padding: UiRect::bottom(px(9)), ..default() }))
                .with_children(|t| {
                    t.spawn((
                        TabText(i),
                        Text::new(b.name.clone()),
                        theme::font(22.0, FontWeight::NORMAL),
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.5)),
                        TextLayout::no_wrap(),
                        Pickable::IGNORE,
                    ));
                    t.spawn((
                        TabUnderline(i),
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(0),
                            right: px(0),
                            bottom: px(0),
                            height: px(3),
                            border_radius: BorderRadius::all(px(2)),
                            display: Display::None,
                            ..default()
                        },
                        BackgroundColor(b.color),
                        BoxShadow(vec![shadow(alpha(b.color, 0.9), 0.0, 14.0, 0.0)]),
                        Pickable::IGNORE,
                    ));
                });
        }
    });
}

pub fn animate_tabs(
    dash: Res<Dash>,
    mut texts: Query<(&TabText, &mut TextColor, &mut TextFont)>,
    mut lines: Query<(&TabUnderline, &mut Node)>,
) {
    for (t, mut color, mut font) in &mut texts {
        let on = t.0 == dash.current;
        set_color(&mut color, TextColor(Color::srgba(1.0, 1.0, 1.0, if on { 1.0 } else { 0.5 })));
        let w = if on { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
        if font.weight != w {
            font.weight = w;
        }
    }
    for (u, mut node) in &mut lines {
        set_display(&mut node, u.0 == dash.current);
    }
}

// ---------- shelf ----------

/// Respawns the tiles whenever the category or its entries change.
pub fn sync_shelf(
    mut commands: Commands,
    dash: Res<Dash>,
    lib: Res<Library>,
    mut shimmer_mats: ResMut<Assets<ShimmerMaterial>>,
    layer: Query<Entity, With<ShelfLayer>>,
    mut anim: ResMut<ShelfAnim>,
    mut last: Local<Option<(usize, u64)>>,
) {
    let key = (dash.current, dash.content_gen);
    if *last == Some(key) {
        return;
    }
    let Ok(layer) = layer.single() else { return };
    let same_blade = last.is_some_and(|(b, _)| b == dash.current);
    *last = Some(key);
    commands.entity(layer).despawn_children();
    if dash.in_grid() {
        return;
    }

    let blade = dash.blade();
    let sel = dash.sel[dash.current];
    let first = sel.saturating_sub(SHELF_LEAD);
    anim.scroll = first as f32 * (TILE_W + TILE_GAP);
    let recent = blade.key == BladeKey::Recent;

    commands.entity(layer).with_children(|shelf| {
        for (i, e) in blade.entries.iter().enumerate() {
            let game = game_of(&lib, e);
            // Stagger the rise for tiles on screen; refreshes of the same category don't animate.
            let appear = if same_blade { 1.0 } else { -0.045 * i.saturating_sub(first) as f32 };
            shelf
                .spawn((
                    Tile { idx: i, sel: if i == sel { 1.0 } else { 0.0 }, vel: 0.0, appear },
                    Hovered::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(TILE_W),
                        height: px(TILE_H),
                        border_radius: BorderRadius::all(px(CARD_R)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Outline { width: px(3), offset: px(0), color: Color::NONE },
                    BoxShadow(vec![
                        shadow(Color::NONE, 0.0, 28.0, 0.0),
                        shadow(Color::srgba(0.0, 0.0, 0.0, 0.6), 16.0, 34.0, 0.0),
                    ]),
                    BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.9)),
                    UiTransform::default(),
                    // Transformed children escape the parent's stacking; pin them explicitly.
                    GlobalZIndex(6),
                ))
                .with_children(|t| {
                    // Fallback card: system entries (icon), games still loading, and games
                    // whose generated cover is on its way (title over a scrim).
                    t.spawn((
                        TileFallback(i),
                        Node {
                            position_type: PositionType::Absolute,
                            width: percent(100),
                            height: percent(100),
                            flex_direction: FlexDirection::Column,
                            justify_content: if game.is_some() { JustifyContent::FlexEnd } else { JustifyContent::Center },
                            align_items: if game.is_some() { AlignItems::FlexStart } else { AlignItems::Center },
                            padding: UiRect::all(px(16)),
                            row_gap: px(14),
                            border_radius: BorderRadius::all(px(CARD_R)),
                            display: if game.is_some() { Display::None } else { Display::Flex },
                            ..default()
                        },
                        if game.is_some() {
                            // Transparent except the title's scrim: whatever art the
                            // tile has (real or generated) shows through.
                            BackgroundGradient::from(LinearGradient {
                                angle: LinearGradient::TO_BOTTOM,
                                stops: vec![
                                    ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.0), percent(0)),
                                    ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.0), percent(55)),
                                    ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.55), percent(100)),
                                ],
                                ..default()
                            })
                        } else {
                            grad(LinearGradient::TO_BOTTOM, Color::srgba(0.22, 0.26, 0.32, 0.85), Color::srgba(0.08, 0.10, 0.13, 0.9))
                        },
                        Pickable::IGNORE,
                    ))
                    .with_children(|f| {
                        if let Some(glyph) = e.glyph {
                            f.spawn((Text::new(glyph.to_string()), theme::icon_font(52.0), TextColor(WHITE), Pickable::IGNORE));
                        }
                        f.spawn((
                            Text::new(e.title.clone()),
                            theme::font(if game.is_some() { 22.0 } else { 17.0 }, FontWeight::SEMIBOLD),
                            TextColor(WHITE),
                            TextLayout::justify(if game.is_some() { Justify::Left } else { Justify::Center }),
                            Pickable::IGNORE,
                        ));
                    });
                    if let Some(g) = game {
                        t.spawn((
                            TileImage(i),
                            ImageNode::default(),
                            Node {
                                position_type: PositionType::Absolute,
                                width: percent(100),
                                height: percent(100),
                                border_radius: BorderRadius::all(px(CARD_R)),
                                display: Display::None,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                        // A light sweep while the cover is still on its way.
                        t.spawn((
                            TileLoading(i),
                            MaterialNode(shimmer_mats.add(ShimmerMaterial {
                                params: Vec4::new(
                                    (fx::hash(&g.id) % 4096) as f32 / 4096.0,
                                    0.0,
                                    0.0,
                                    0.0,
                                ),
                            })),
                            Node {
                                position_type: PositionType::Absolute,
                                width: percent(100),
                                height: percent(100),
                                border_radius: BorderRadius::all(px(CARD_R)),
                                display: Display::None,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                    }
                    if let (true, Some(g)) = (recent, game) {
                        t.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(10),
                                bottom: px(10),
                                padding: UiRect::axes(px(8), px(3)),
                                border_radius: BorderRadius::all(px(5)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.68)),
                            Pickable::IGNORE,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new(g.system.label().to_uppercase()),
                                theme::font(11.0, FontWeight::BOLD),
                                // Small caps breathe better with a hair of air
                                // (the mockup's 1px on the badge).
                                theme::tracking(1.0),
                                TextColor(WHITE),
                                Pickable::IGNORE,
                            ));
                        });
                    }
                    t.spawn((
                        TileDim(i),
                        Node {
                            position_type: PositionType::Absolute,
                            width: percent(100),
                            height: percent(100),
                            border_radius: BorderRadius::all(px(CARD_R)),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        Pickable::IGNORE,
                    ));
                });
            // Its reflection on the glossy floor below the shelf.
            if game.is_some() {
                shelf
                    .spawn((
                        TileReflection(i),
                        Node {
                            position_type: PositionType::Absolute,
                            top: px(TILE_SEL_H + REFL_GAP),
                            width: px(TILE_W),
                            height: px(REFL_H),
                            overflow: Overflow::clip(),
                            border_radius: BorderRadius::all(px(CARD_R)),
                            display: Display::None,
                            ..default()
                        },
                        GlobalZIndex(5),
                        Pickable::IGNORE,
                    ))
                    .with_children(|r| {
                        r.spawn((
                            TileReflectionImage(i),
                            ImageNode { flip_y: true, ..default() },
                            Node {
                                position_type: PositionType::Absolute,
                                width: percent(100),
                                height: percent(100),
                                border_radius: BorderRadius::all(px(CARD_R)),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                        r.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                width: percent(100),
                                height: percent(100),
                                border_radius: BorderRadius::all(px(CARD_R)),
                                ..default()
                            },
                            BackgroundGradient::from(LinearGradient {
                                angle: LinearGradient::TO_BOTTOM,
                                stops: vec![
                                    ColorStop::new(Color::srgba(0.012, 0.02, 0.031, 0.4), percent(0)),
                                    ColorStop::new(Color::srgb(0.012, 0.02, 0.031), percent(62)),
                                ],
                                ..default()
                            }),
                            Pickable::IGNORE,
                        ));
                    });
            }
        }
    });
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn animate_shelf(
    dash: Res<Dash>,
    lib: Res<Library>,
    mut tex: ResMut<Textures>,
    time: Res<Time>,
    mut fx: ResMut<Fx>,
    scale: Res<UiScale>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut anim: ResMut<ShelfAnim>,
    mut tiles: Query<(&mut Tile, &mut Node, &mut Outline, &mut BoxShadow, &mut UiTransform)>,
    mut images: Query<(&TileImage, &mut ImageNode, &mut Node), Without<Tile>>,
    mut fallbacks: Query<(&TileFallback, &mut Node), (Without<Tile>, Without<TileImage>)>,
    mut shimmers: Query<
        (&TileLoading, &mut Node),
        (Without<Tile>, Without<TileImage>, Without<TileFallback>, Without<TileReflection>),
    >,
    mut dims: Query<(&TileDim, &mut BackgroundColor)>,
    mut refls: Query<
        (&TileReflection, &mut Node),
        (Without<Tile>, Without<TileImage>, Without<TileFallback>),
    >,
    mut refl_images: Query<(&TileReflectionImage, &mut ImageNode), Without<TileImage>>,
) {
    let dt = time.delta_secs().min(1.0 / 20.0);
    let now = time.elapsed_secs();
    let blade = dash.blade();
    let sel = dash.sel[dash.current];
    let accent = fx.accent_color();
    let launch = fx.launch_env();
    let target = sel.saturating_sub(SHELF_LEAD) as f32 * (TILE_W + TILE_GAP);
    anim.scroll = approach(anim.scroll, target, 14.0, dt);

    // Selection weight per tile is a springy value: the newly selected cover pops a little
    // past full size and settles. Then lay out left to right.
    let mut state: Vec<(usize, f32, f32)> = Vec::new(); // (idx, sel, appear)
    for (mut t, ..) in &mut tiles {
        let want = if t.idx == sel { 1.0 } else { 0.0 };
        for _ in 0..2 {
            let h = dt * 0.5;
            let acc = 210.0 * (want - t.sel) - 15.0 * t.vel;
            t.vel += acc * h;
            t.sel += t.vel * h;
        }
        if (t.sel - want).abs() < 1e-3 && t.vel.abs() < 1e-3 {
            t.sel = want;
            t.vel = 0.0;
        }
        t.appear = (t.appear + dt * 3.2).min(1.0);
        state.push((t.idx, t.sel, t.appear));
    }
    state.sort_by_key(|s| s.0);
    let mut xs = Vec::with_capacity(state.len());
    let mut x = SIDE - anim.scroll;
    for &(_, s, ..) in &state {
        xs.push(x);
        x += TILE_W + (TILE_SEL_W - TILE_W) * s.max(0.0) + TILE_GAP;
    }
    let lookup = |idx: usize| state.iter().position(|s| s.0 == idx);
    let size_of = |s: f32| (TILE_W + (TILE_SEL_W - TILE_W) * s, TILE_H + (TILE_SEL_H - TILE_H) * s);

    let breathe = 0.85 + 0.15 * (now * 2.4).sin();
    let ring_color = Color::from(accent.to_srgba().mix(&Srgba::WHITE, 0.35));
    let press = fx.press_env();
    for (t, mut node, mut outline, mut shadows, mut tf) in &mut tiles {
        let Some(k) = lookup(t.idx) else { continue };
        let s = t.sel;
        let sc = s.clamp(0.0, 1.0);
        let (w, h) = size_of(s);
        set_px(&mut node, |n| &mut n.left, xs[k].round());
        set_px(&mut node, |n| &mut n.top, (TILE_SEL_H - h).round());
        set_px(&mut node, |n| &mut n.width, w.round());
        set_px(&mut node, |n| &mut n.height, h.round());
        let ring = alpha(ring_color, sc);
        if outline.color != ring {
            outline.color = ring;
        }
        // Glow in the game's own colour, breathing; it flares when the game launches
        // and spikes briefly when you press A on it.
        let is_sel = t.idx == sel;
        let flare = if is_sel { launch } else { 0.0 };
        let dip = if is_sel { press } else { 0.0 };
        let glow = alpha(accent, (0.55 * sc * breathe + 0.45 * flare + 0.30 * dip).min(1.0));
        if shadows.0[0].color != glow {
            shadows.0[0].color = glow;
            shadows.0[0].blur_radius = px(28.0 + 40.0 * flare);
            shadows.0[0].spread_radius = px(30.0 * flare);
        }
        let e = ease(t.appear);
        let pop = 1.0 + 0.10 * flare - 0.045 * dip;
        // The row leans into the move and rights itself as the scroll settles.
        let lag = target - anim.scroll;
        let want = UiTransform {
            translation: Val2::px(0.0, (1.0 - e) * 60.0 - 14.0 * flare - 10.0 * dip),
            scale: Vec2::splat(pop),
            rotation: Rot2::radians(0.040 * (lag / 130.0).tanh()),
            ..default()
        };
        if *tf != want {
            *tf = want;
        }
    }

    // A launch bursts from the selected cover: keep its exact spot (canvas → uv).
    if let Some(k) = lookup(sel) {
        let (w, h) = size_of(state[k].1);
        let canvas_w = windows
            .single()
            .map(|w| w.width() / scale.0.max(0.01))
            .unwrap_or(1600.0);
        fx.origin = Vec2::new((xs[k] + w * 0.5) / canvas_w, (CANVAS_H - SHELF_BOTTOM - h * 0.5) / CANVAS_H);
    }

    // Box art, generated covers, shimmer, dimming and reflections.
    let mut missing = Vec::new();
    let mut loading = Vec::new();
    let mut covers: Vec<(usize, Handle<Image>, UVec2)> = Vec::new();
    for (ti, mut img, mut node) in &mut images {
        let Some(game) = blade.entries.get(ti.0).and_then(|e| game_of(&lib, e)) else { continue };
        let fade = lookup(ti.0).map(|k| ease(state[k].2)).unwrap_or(1.0);
        match tex.get(&game.id, ArtKind::Cover) {
            Some((h, size)) => {
                if img.image != h {
                    img.image = h.clone();
                }
                let c = Color::srgba(1.0, 1.0, 1.0, fade);
                if img.color != c {
                    img.color = c;
                }
                set_display(&mut node, true);
                covers.push((ti.0, h, size));
            }
            None => {
                set_display(&mut node, false);
                if tex.is_missing(&game.id, ArtKind::Cover) {
                    missing.push(ti.0);
                    // No art anywhere: show the generated cover (procedural.rs), so the
                    // tile is still a piece of the library, reflection included.
                    if let Some(fb) = tex.fallback(&game.id, system_color(game.system)) {
                        if img.image != fb {
                            img.image = fb.clone();
                        }
                        let c = Color::srgba(1.0, 1.0, 1.0, fade);
                        if img.color != c {
                            img.color = c;
                        }
                        set_display(&mut node, true);
                        covers.push((ti.0, fb, UVec2::new(super::procedural::W, super::procedural::H)));
                    }
                } else {
                    loading.push(ti.0);
                }
            }
        }
    }
    for (f, mut node) in &mut fallbacks {
        let is_game = blade.entries.get(f.0).and_then(|e| game_of(&lib, e)).is_some();
        set_display(&mut node, !is_game || missing.contains(&f.0) || loading.contains(&f.0));
    }
    for (s, mut node) in &mut shimmers {
        set_display(&mut node, loading.contains(&s.0));
    }
    for (d, mut bg) in &mut dims {
        let (s, fade) = lookup(d.0).map(|k| (state[k].1.clamp(0.0, 1.0), ease(state[k].2))).unwrap_or((0.0, 1.0));
        // Unselected tiles step back; everything fades in on arrival.
        let a = (0.55 * (1.0 - s)).max(1.0 - fade);
        set_color(&mut bg, BackgroundColor(Color::srgba(0.01, 0.015, 0.025, a)));
    }
    for (r, mut node) in &mut refls {
        let Some(k) = lookup(r.0) else { continue };
        // Hidden during the launch flash, where the dark floor fade would show as boxes.
        let has_cover = covers.iter().any(|c| c.0 == r.0);
        set_display(&mut node, has_cover && launch < 0.02);
        let (w, _) = size_of(state[k].1);
        set_px(&mut node, |n| &mut n.left, xs[k].round());
        set_px(&mut node, |n| &mut n.width, w.round());
    }
    for (r, mut img) in &mut refl_images {
        let Some(k) = lookup(r.0) else { continue };
        let Some((_, h, size)) = covers.iter().find(|c| c.0 == r.0) else { continue };
        if img.image != *h {
            img.image = h.clone();
        }
        // Mirror the bottom strip of the cover at the tile's current scale.
        let (_, th) = size_of(state[k].1);
        let frac = (REFL_H / th).min(1.0);
        let rect = Rect::new(0.0, size.y as f32 * (1.0 - frac), size.x as f32, size.y as f32);
        if img.rect != Some(rect) {
            img.rect = Some(rect);
        }
        let s = state[k].1.clamp(0.0, 1.0);
        let fade = ease(state[k].2);
        let c = Color::srgba(1.0, 1.0, 1.0, (0.28 + 0.30 * s) * fade * fade);
        if img.color != c {
            img.color = c;
        }
    }
}

// ---------- info block ----------

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_info(
    dash: Res<Dash>,
    lib: Res<Library>,
    mut tex: ResMut<Textures>,
    time: Res<Time>,
    mut anim: ResMut<InfoAnim>,
    fx: Res<Fx>,
    mut logo: Query<(&mut ImageNode, &mut Node), With<LogoImg>>,
    mut block: Query<&mut UiTransform, With<InfoBlock>>,
    mut details_btn: Query<&mut Node, (With<DetailsBtn>, Without<LogoImg>)>,
    mut play_btn: Query<(&mut BoxShadow, &mut BorderColor, &mut UiTransform), (With<PlayBtn>, Without<InfoBlock>)>,
    mut slots: Query<(&Slot, &mut Text, &mut TextColor)>,
) {
    let blade = dash.blade();
    let sel = dash.sel[dash.current];
    let entry = dash.selected().cloned();
    let game = entry.as_ref().and_then(|e| game_of(&lib, e)).cloned();

    let key = (dash.current, sel, dash.content_gen);
    if anim.key != key {
        anim.key = key;
        anim.t = 0.0;
    }
    anim.t = (anim.t + time.delta_secs() * 4.0).min(1.0);
    let e = ease(anim.t);

    if let Ok(mut tf) = block.single_mut() {
        let want = UiTransform { translation: Val2::px(-16.0 * (1.0 - e), 0.0), ..default() };
        if *tf != want {
            *tf = want;
        }
    }

    let logo_tex = game.as_ref().and_then(|g| tex.get(&g.id, ArtKind::Logo));
    let logo_missing = game.as_ref().is_none_or(|g| tex.is_missing(&g.id, ArtKind::Logo));
    if let Ok((mut img, mut node)) = logo.single_mut() {
        match &logo_tex {
            Some((h, size)) => {
                let s = (560.0 / size.x as f32).min(190.0 / size.y as f32);
                if img.image != *h {
                    img.image = h.clone();
                }
                set_px(&mut node, |n| &mut n.width, (size.x as f32 * s).round());
                set_px(&mut node, |n| &mut n.height, (size.y as f32 * s).round());
                let c = Color::srgba(1.0, 1.0, 1.0, e);
                if img.color != c {
                    img.color = c;
                }
                set_display(&mut node, true);
            }
            None => set_display(&mut node, false),
        }
    }

    // Big title when there's no logo (or for system entries).
    let title = match (&game, &entry) {
        (Some(g), _) if logo_tex.is_none() && logo_missing => g.title.clone(),
        (None, Some(en)) => en.title.clone(),
        _ => String::new(),
    };
    let meta = match (&game, &entry) {
        (Some(g), _) => game_meta(g),
        (None, Some(en)) => en.detail.clone(),
        _ => String::new(),
    };
    let play_label = match entry.as_ref().map(|e| e.action) {
        Some(Action::Launch(_)) => "Play",
        Some(Action::OpenWishlist(_)) => "Open page",
        Some(Action::Exit) => "Exit",
        Some(Action::OpenConfig) => "Open",
        Some(Action::ToggleAutostart | Action::ToggleSounds) => "Toggle",
        Some(_) => "Start",
        None => "",
    };
    let count = if blade.entries.is_empty() { String::new() } else { format!("{} / {}", sel + 1, blade.entries.len()) };
    for (slot, mut text, mut color) in &mut slots {
        match slot {
            Slot::LogoText => {
                set_text(&mut text, &title);
                set_color(&mut color, TextColor(Color::srgba(1.0, 1.0, 1.0, e)));
            }
            Slot::Meta => {
                set_text(&mut text, &meta);
                set_color(&mut color, TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8 * e)));
            }
            Slot::PlayLabel => set_text(&mut text, play_label),
            Slot::Count => set_text(&mut text, &count),
            _ => {}
        }
    }
    if let Ok(mut node) = details_btn.single_mut() {
        set_display(&mut node, game.is_some());
    }
    if let Ok((mut sh, mut border, mut tf)) = play_btn.single_mut() {
        let edge = BorderColor::all(alpha(fx.accent_color(), 0.85));
        if *border != edge {
            *border = edge;
        }
        let pulse = 0.26 + 0.10 * (time.elapsed_secs() * 2.2).sin() + 0.6 * fx.launch_env();
        let c = alpha(fx.accent_color(), pulse.min(1.0));
        if sh.0[0].color != c {
            sh.0[0].color = c;
        }
        // The pill takes the A-press with the tile: a small, physical dip.
        let want = UiTransform { scale: Vec2::splat(1.0 - 0.05 * fx.press_env()), ..default() };
        if *tf != want {
            *tf = want;
        }
    }
}

// ---------- background ----------

pub fn update_background(
    dash: Res<Dash>,
    lib: Res<Library>,
    mut tex: ResMut<Textures>,
    mut fade: ResMut<HeroFade>,
    fx: Res<Fx>,
    time: Res<Time>,
    mut mats: ResMut<Assets<BgMaterial>>,
    mut layout: ResMut<super::particles::HeroLayout>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut bg: Query<(&MeshMaterial2d<BgMaterial>, &mut Transform), With<BgNode>>,
    mut backdrop: Local<f32>,
    mut splash: Local<f32>,
) {
    let dt = time.delta_secs();
    // The All grid shows the image as a full-screen backdrop instead of across the top.
    *backdrop = approach(*backdrop, if dash.in_grid() { 1.0 } else { 0.0 }, 7.0, dt.min(0.05));
    let fill = ease(*backdrop);
    // The launch splash: the picture swells to full-bleed and whites out (see fx.rs
    // for the timeline; the chrome side is update_launch_splash).
    *splash = approach(*splash, fx.launch_fill(), 16.0, dt.min(0.05));
    let want_id = dash.selected().and_then(|e| game_of(&lib, e)).map(|g| g.id.clone());
    let current_id = fade.b.as_ref().map(|s| s.id.clone());
    // A move this frame gives the hero change a direction: the new picture comes in
    // from the side you flicked, the old one gives way.
    let dir = fx.move_dir();
    if want_id != current_id {
        match &want_id {
            Some(id) => {
                if let Some((image, size)) = tex.get(id, ArtKind::Hero) {
                    fade.a = fade.b.take();
                    let scene = tex.scene(id);
                    fade.b = Some(HeroSlot { id: id.clone(), image, size, scene, age: 0.0 });
                    fade.t = 0.0;
                    fade.dir = dir;
                } else if tex.is_missing(id, ArtKind::Hero) && fade.b.is_some() {
                    fade.a = fade.b.take();
                    fade.t = 0.0;
                    fade.dir = dir;
                }
            }
            None => {
                if fade.b.is_some() {
                    fade.a = fade.b.take();
                    fade.t = 0.0;
                    fade.dir = dir;
                }
            }
        }
    } else if let Some(id) = current_id {
        // Keeps it resident; a different handle means the art was just improved on disk,
        // so cross-fade to the new version.
        if let Some((image, size)) = tex.get(&id, ArtKind::Hero) {
            if fade.b.as_ref().is_some_and(|b| b.image != image) {
                let scene = tex.scene(&id);
                fade.a = fade.b.take();
                fade.b = Some(HeroSlot { id: id.clone(), image, size, scene, age: 0.0 });
                fade.t = 0.0;
            }
        }
        if let Some(b) = fade.b.as_mut().filter(|b| b.scene.is_none()) {
            b.scene = tex.scene(&id);
        }
    }
    fade.t = (fade.t + dt / 0.8).min(1.0);
    let f = &mut *fade;
    for s in [&mut f.a, &mut f.b].into_iter().flatten() {
        s.age += dt;
    }

    let Ok((handle, mut tf)) = bg.single_mut() else { return };
    let screen = windows.single().map(|w| Vec2::new(w.width(), w.height())).unwrap_or(Vec2::new(1600.0, 900.0));
    let scale = screen.extend(1.0);
    if tf.scale != scale {
        tf.scale = scale;
    }
    let Some(mut mat) = mats.get_mut(&handle.0) else { return };
    mat.tint = fx.accent.to_vec4();
    let t = fade.t * fade.t * (3.0 - 2.0 * fade.t);
    let has = |s: &Option<HeroSlot>| if s.is_some() { 1.0 } else { 0.0 };
    mat.params = Vec4::new(time.elapsed_secs(), t, has(&fade.a), has(&fade.b));
    let size = |s: &Option<HeroSlot>| s.as_ref().map(|x| x.size.as_vec2()).unwrap_or(Vec2::ONE);
    // A new hero lands slightly zoomed in and settles, then moves in its game's own style.
    // Depth is in flat layers (cut-out cards), so subjects move rigidly and never bend.
    let motion = |s: &Option<HeroSlot>| -> HeroMotion {
        s.as_ref().map(|x| hero_motion(&x.id, x.age, fx.cam)).unwrap_or_default()
    };
    let (mut ma, mut mb) = (motion(&fade.a), motion(&fade.b));
    // Directional change: the new scene comes in from the side you moved and settles,
    // the old one slides out of its way. Cam units are small (see background.wgsl);
    // this stays inside the overscan margin.
    let step = 5.0 * fade.dir;
    ma.cam.x += step * super::fx::smoothstep(fade.t, 0.0, 1.0);
    if let Some(b) = fade.b.as_ref() {
        mb.cam.x -= step * (-(b.age.max(0.0)) * 4.0).exp();
    }
    let focal = |s: &Option<HeroSlot>| s.as_ref().and_then(|x| x.scene.as_ref()).map(|d| d.focal).unwrap_or(Vec2::new(0.5, 0.4));
    let has_depth = |s: &Option<HeroSlot>| if s.as_ref().is_some_and(|x| x.scene.is_some()) { 1.0 } else { 0.0 };
    let (fa, fb) = (focal(&fade.a), focal(&fade.b));
    mat.breath = Vec4::new(ma.dolly, mb.dolly, ma.relight, mb.relight);
    mat.motion = Vec4::new(ma.cam.x, ma.cam.y, mb.cam.x, mb.cam.y);
    mat.focal = Vec4::new(fa.x, fa.y, fb.x, fb.y);
    mat.look = Vec4::new(fx.cam.x, fx.cam.y, has_depth(&fade.a), has_depth(&fade.b));
    let l = fx.look;
    // Art that makes its own particles (embers, glints, stars, leaves) doesn't also get the
    // generic overlay.
    let own = fade.b.as_ref().and_then(|x| x.scene.as_ref()).is_some_and(|s| !s.emitters.is_empty());
    let overlay = if own { 0.0 } else { l.style as f32 };
    mat.style = Vec4::new(overlay, fx.look_amount, l.seed, l.beam_tint);
    mat.beams = Vec4::new(l.beams.x, l.beams.y, fx.focus, 0.0);
    let scene = |s: &Option<HeroSlot>| s.as_ref().and_then(|x| x.scene.clone());
    let (sca, scb) = (scene(&fade.a), scene(&fade.b));
    let da = sca.as_ref().map(|d| d.depth.clone()).unwrap_or_default();
    let db = scb.as_ref().map(|d| d.depth.clone()).unwrap_or_default();
    let xa = sca.as_ref().map(|d| d.fxmap.clone()).unwrap_or_default();
    let xb = scb.as_ref().map(|d| d.fxmap.clone()).unwrap_or_default();
    if mat.depth_a != da {
        mat.depth_a = da;
    }
    if mat.depth_b != db {
        mat.depth_b = db;
    }
    if mat.fxmap_a != xa {
        mat.fxmap_a = xa;
    }
    if mat.fxmap_b != xb {
        mat.fxmap_b = xb;
    }
    let (sa, sb) = (size(&fade.a), size(&fade.b));
    mat.sizes = Vec4::new(sa.x, sa.y, sb.x, sb.y);
    let physical = windows
        .single()
        .map(|w| Vec2::new(w.physical_width() as f32, w.physical_height() as f32))
        .unwrap_or(screen);
    // The launch swell leans into the screen as it fills it.
    let zpush = 1.0 + 0.09 * *splash;
    mat.zoom = Vec4::new(ma.zoom * zpush, mb.zoom * zpush, physical.x, physical.y);
    // x: grid backdrop, y: launch swell, z: launch white-out.
    mat.layout = Vec4::new(fill, *splash, fx.launch_flash(), 0.0);

    // Tell the particles where each image is (current one last). While an overlay holds the
    // focus, the picture behind it is blurred and dim — its particles fade out too.
    layout.slots.clear();
    for (s, m, w) in [(&fade.a, &ma, has(&fade.a) * (1.0 - t)), (&fade.b, &mb, t)] {
        let Some(slot) = s else { continue };
        let Some(sc) = slot.scene.as_ref() else { continue };
        layout.slots.push(super::particles::SlotView {
            id: slot.id.clone(),
            tex: slot.size.as_vec2(),
            zoom: m.zoom,
            focal: sc.focal,
            dolly: m.dolly,
            cam: m.cam,
            weight: w * (1.0 - fx.focus),
            fill,
            emitters: sc.emitters.clone(),
        });
    }
    mat.fx = Vec4::new(fx.launch_env(), fx.kick, fx.origin.x, fx.origin.y);
    let ha = fade.a.as_ref().map(|x| x.image.clone()).unwrap_or_default();
    let hb = fade.b.as_ref().map(|x| x.image.clone()).unwrap_or_default();
    if mat.hero_a != ha {
        mat.hero_a = ha;
    }
    if mat.hero_b != hb {
        mat.hero_b = hb;
    }
}

/// How a hero image moves right now.
#[derive(Default)]
struct HeroMotion {
    zoom: f32,
    /// Depth dolly: nearer layers magnify by this much (times their depth).
    dolly: f32,
    /// Parallax camera; layers shift by it according to their depth.
    cam: Vec2,
    /// 0..1, light swelling on near subjects.
    relight: f32,
}

/// Each game gets one of a few motion styles (picked from its id), so the shelf doesn't feel
/// like the same animation on every picture.
fn hero_motion(id: &str, age: f32, look: Vec2) -> HeroMotion {
    use std::f32::consts::TAU;
    let wave = |period: f32| 0.5 - 0.5 * (age * TAU / period).cos();
    let settle = 1.0 + 0.03 * (-age * 3.0).exp();
    match super::fx::hash(id) % 3 {
        // Breathe: the near layers swell toward you and relax, like a slow inhale.
        0 => HeroMotion { zoom: settle * (1.0 + 0.01 * wave(11.0)), dolly: 0.012 * wave(11.0), cam: look * 0.3, relight: wave(11.0) },
        // Glide: the camera slides gently side to side and the layers separate.
        1 => {
            let side = 0.8 * (age * TAU / 32.0).sin();
            let lift = 0.25 * (age * TAU / 45.0).sin();
            HeroMotion { zoom: settle * 1.02, dolly: 0.0, cam: Vec2::new(side, lift) + look * 0.3, relight: wave(14.0) * 0.6 }
        }
        // Push-in: a very slow zoom into the subject and back out.
        _ => HeroMotion { zoom: settle * (1.0 + 0.03 * wave(50.0)), dolly: 0.005 * wave(50.0), cam: look * 0.25, relight: wave(50.0) },
    }
}

// ---------- chrome & overlays ----------

fn details_text(g: &Game) -> String {
    if let Launch::Url { url } = &g.launch {
        return format!(
            "STATUS\nNot downloaded yet\n\nSOURCE\n{}\n{url}\n\nID (for config overrides)\n{}",
            crate::library::wishlist::source_host(url),
            g.id
        );
    }
    let played = g.last_played.map(super::state::ago).unwrap_or_else(|| "Never".into());
    let controller = match g.controller {
        Some(ControllerSupport::Full) => {
            "Full support — plays start to finish on a gamepad.".to_string()
        }
        Some(ControllerSupport::Partial) => {
            "Partial — you may still need a keyboard and mouse for some menus or actions.".into()
        }
        Some(ControllerSupport::None) => {
            "None listed — expect keyboard and mouse. Steam Input may still make it playable.".into()
        }
        None => format!(
            "Unknown — still being looked up, or nothing was found.\n\
             If you know better, set controller = \"full\" under [overrides.{}] in config.toml.",
            g.id
        ),
    };
    let (location, how) = match &g.launch {
        Launch::Steam { appid, install_dir } => (install_dir.display().to_string(), format!("Steam (app {appid})")),
        Launch::Url { url } => (url.clone(), "Wishlist page".into()),
        Launch::Exe { exe, args, watch_dir, .. } => {
            let loc = watch_dir.clone().unwrap_or_else(|| {
                args.iter().find(|a| a.contains('\\')).map(std::path::PathBuf::from).unwrap_or_else(|| exe.clone())
            });
            let how = match g.system {
                System::Xbox | System::Xbox360 => exe.file_name().unwrap_or_default().to_string_lossy().to_string(),
                _ => exe.display().to_string(),
            };
            (loc.display().to_string(), how)
        }
    };
    format!(
        "SYSTEM\n{}\n\nCONTROLLER\n{controller}\n\nLAST PLAYED\n{played}\n\nLOCATION\n{location}\n\nLAUNCHES WITH\n{how}\n\nID (for config overrides)\n{}",
        g.system.label(),
        g.id
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_chrome(
    mut dash: ResMut<Dash>,
    lib: Res<Library>,
    nav: Res<NavInput>,
    time: Res<Time>,
    running: Res<super::Running>,
    fx: Res<Fx>,
    mut slots: Query<(&Slot, &mut Text, &mut TextColor)>,
    mut nodes: ParamSet<(
        Query<&mut Node, With<GuideOverlay>>,
        Query<&mut Node, With<Toast>>,
        Query<&mut Node, With<RunningOverlay>>,
        Query<&mut Node, With<DetailsSheet>>,
        Query<&mut Node, With<Fader>>,
    )>,
    mut guide_rows: Query<(&GuideRow, &mut BackgroundColor), (Without<Fader>, Without<RunningOverlay>)>,
    mut fader: Query<&mut BackgroundColor, (With<Fader>, Without<RunningOverlay>)>,
    mut running_bg: Query<&mut BackgroundColor, With<RunningOverlay>>,
) {
    // Intro fade from black.
    let t = time.elapsed_secs();
    if let Ok(mut bg) = fader.single_mut() {
        let a = (1.0 - (t - 0.15) / 0.7).clamp(0.0, 1.0);
        set_color(&mut bg, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, a)));
        if let Ok(mut n) = nodes.p4().single_mut() {
            set_display(&mut n, a > 0.0);
        }
    }

    if let Some(PadEvent(on)) = nav.pad_event {
        let msg = if on { "Controller connected" } else { "Controller disconnected" };
        dash.toast = Some((msg.into(), 2.5));
    }

    let mut toast_text = None;
    if let Some((msg, left)) = &mut dash.toast {
        *left -= time.delta_secs();
        if *left <= 0.0 {
            dash.toast = None;
        } else {
            toast_text = Some(msg.clone());
        }
    }
    let selected_game = dash.selected().and_then(|e| game_of(&lib, e)).cloned();
    let show_details = dash.mode == Mode::Details && selected_game.is_some();
    let back = if nav.device == Device::Gamepad { "B" } else { "Backspace" };
    let running_text = running.0.as_ref().map(|r| {
        format!("Now playing\n{}\n\nThe dashboard comes back when the game closes.\nPress {back} to return now.", r.title)
    });
    // After a launch, the splash plays out first (swell, flash, stamp); the "now
    // playing" screen fades in over what's left of the flash.
    let reveal = match fx.launch {
        Some(t) => ((t - 1.15) / 0.5).clamp(0.0, 1.0),
        None => 1.0,
    };
    if let Ok(mut bg) = running_bg.single_mut() {
        set_color(&mut bg, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.93 * reveal)));
    }

    for (slot, mut text, mut color) in &mut slots {
        match slot {
            Slot::Clock => set_text(&mut text, &crate::win::local_time_hm()),
            Slot::Status => set_text(&mut text, &dash.status),
            Slot::Toast => set_text(&mut text, toast_text.as_deref().unwrap_or("")),
            Slot::Running => {
                set_text(&mut text, running_text.as_deref().unwrap_or(""));
                set_color(&mut color, TextColor(Color::srgba(0.97, 0.98, 1.0, reveal)));
            }
            Slot::DetTitle => set_text(&mut text, selected_game.as_ref().map(|g| g.title.as_str()).unwrap_or("")),
            Slot::DetBody => {
                let s = selected_game.as_ref().map(details_text).unwrap_or_default();
                set_text(&mut text, &s);
            }
            _ => {}
        }
    }

    if let Ok(mut n) = nodes.p0().single_mut() {
        set_display(&mut n, dash.mode == Mode::Guide);
    }
    if let Ok(mut n) = nodes.p1().single_mut() {
        set_display(&mut n, toast_text.is_some());
    }
    if let Ok(mut n) = nodes.p2().single_mut() {
        set_display(&mut n, dash.mode == Mode::Running);
    }
    if let Ok(mut n) = nodes.p3().single_mut() {
        set_display(&mut n, show_details);
    }
    for (row, mut bg) in &mut guide_rows {
        let c = if row.0 == dash.guide_sel { Color::srgba(1.0, 1.0, 1.0, 0.14) } else { Color::NONE };
        set_color(&mut bg, BackgroundColor(c));
    }
}

// ---------- launch splash ----------

/// The launch splash's chrome: the dark veil that closes over the interface while
/// the hero sinks beneath it (background.wgsl), and the game's mark stamped into
/// the black — its logo if it has one, otherwise its name, big.
#[allow(clippy::type_complexity)]
pub fn update_launch_splash(
    dash: Res<Dash>,
    lib: Res<Library>,
    mut tex: ResMut<Textures>,
    fx: Res<Fx>,
    mut flash: Query<(&mut Node, &mut BackgroundColor), With<LaunchFlash>>,
    mut stamp: Query<(&mut Node, &mut UiTransform), (With<LaunchStamp>, Without<LaunchFlash>)>,
    mut logo: Query<(&mut ImageNode, &mut Node), (With<StampLogo>, Without<LaunchStamp>, Without<LaunchFlash>, Without<StampText>)>,
    mut title: Query<(&mut Text, &mut TextColor, &mut Node), (With<StampText>, Without<LaunchStamp>, Without<LaunchFlash>, Without<StampLogo>)>,
) {
    let Some(t) = fx.launch else {
        if let Ok((mut n, ..)) = flash.single_mut() {
            set_display(&mut n, false);
        }
        if let Ok((mut n, ..)) = stamp.single_mut() {
            set_display(&mut n, false);
        }
        return;
    };
    let flash_a = fx.launch_flash();
    let stamp_a = fx::smoothstep(t, 0.50, 0.78) * (1.0 - fx::smoothstep(t, 1.05, 1.65));

    if let Ok((mut n, mut bg)) = flash.single_mut() {
        set_display(&mut n, flash_a > 0.0);
        // A shutter of black, not a flash of light: the game takes over in the dark.
        set_color(&mut bg, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, flash_a)));
    }

    let game = dash.selected().and_then(|e| game_of(&lib, e)).cloned();
    let logo_tex = game.as_ref().and_then(|g| tex.get(&g.id, ArtKind::Logo));
    if let Ok((mut n, mut tf)) = stamp.single_mut() {
        set_display(&mut n, stamp_a > 0.0);
        // Stamps land a touch large and settle, like a seal pressed into the dark.
        let pop = 0.94 + 0.06 * fx::smoothstep(t, 0.50, 0.92);
        let want = UiTransform { scale: Vec2::splat(pop), ..default() };
        if *tf != want {
            *tf = want;
        }
    }
    if let Ok((mut img, mut node)) = logo.single_mut() {
        match &logo_tex {
            Some((h, size)) => {
                let s = (640.0 / size.x as f32).min(340.0 / size.y as f32);
                if img.image != *h {
                    img.image = h.clone();
                }
                set_px(&mut node, |n| &mut n.width, (size.x as f32 * s).round());
                set_px(&mut node, |n| &mut n.height, (size.y as f32 * s).round());
                let c = Color::srgba(1.0, 1.0, 1.0, stamp_a);
                if img.color != c {
                    img.color = c;
                }
                set_display(&mut node, true);
            }
            None => set_display(&mut node, false),
        }
    }
    if let Ok((mut text, mut color, mut node)) = title.single_mut() {
        set_text(&mut text, game.as_ref().map(|g| g.title.as_str()).unwrap_or(""));
        set_color(&mut color, TextColor(Color::srgba(1.0, 1.0, 1.0, stamp_a)));
        set_display(&mut node, logo_tex.is_none() as bool);
    }
}

// ---------- device prompts ----------

/// Flips every prompt glyph to the device in use, cascading left to right.
pub fn animate_prompts(
    nav: Res<NavInput>,
    time: Res<Time>,
    mut swaps: Query<(&mut DeviceSwap, &mut UiTransform, &Children)>,
    mut variants: Query<(&ForDevice, &mut Node)>,
) {
    let dt = time.delta_secs().min(0.05);
    for (mut s, mut tf, children) in &mut swaps {
        let mut apply = false;
        if s.fresh {
            s.fresh = false;
            s.want = nav.device;
            s.shown = nav.device;
            s.t = 1.0;
            apply = true;
        } else if s.want != nav.device {
            s.want = nav.device;
            if s.t >= 1.0 {
                s.t = -0.045 * s.order;
            } else if s.t > 0.5 {
                // Already showing the other side: close again from where it is.
                s.t = 1.0 - s.t;
            }
        }
        if s.t < 1.0 {
            s.t = (s.t + dt / 0.26).min(1.0);
        }
        if s.t >= 0.5 && s.shown != s.want {
            s.shown = s.want;
            apply = true;
        }
        if apply {
            for c in children.iter() {
                if let Ok((v, mut node)) = variants.get_mut(c) {
                    set_display(&mut node, v.0 == s.shown);
                }
            }
        }
        let k = if s.t <= 0.0 { 1.0 } else { (std::f32::consts::PI * s.t).cos().abs() };
        let want = UiTransform { scale: Vec2::new(1.0 + 0.12 * (1.0 - k), k.max(0.001)), ..default() };
        if *tf != want {
            *tf = want;
        }
    }
}
