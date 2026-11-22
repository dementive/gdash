//! The All tab: every game at once in a grid of box art, with the selected game's name above
//! it and its hero image living on in the background, dimmed.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui::{BackgroundGradient, ColorStop, LinearGradient, UiScale};
use bevy::window::PrimaryWindow;

use super::fx::{self, Fx};
use super::glass::ShimmerMaterial;
use super::state::{BladeKey, Dash, Library};
use super::textures::Textures;
use super::theme::{self, *};
use super::view::{InfoBlock, ShelfCount, ShelfLayer, approach, ease, game_meta, game_of, set_color, set_display, set_px, set_text, shadow};
use crate::art::ArtKind;

/// Holds the tiles; clips them below the header.
#[derive(Component)]
pub struct GridLayer;
/// Darkens the hero so the grid reads.
#[derive(Component)]
pub struct GridShade;
#[derive(Component)]
pub struct GridHeader;
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum GridText {
    Title,
    Meta,
    Count,
}

#[derive(Component)]
pub struct GridTile {
    pub idx: usize,
    /// Springy selection weight (pops past 1 and settles).
    sel: f32,
    vel: f32,
    appear: f32,
}
#[derive(Component)]
pub struct GridImage(usize);
#[derive(Component)]
pub struct GridFallback(usize);
/// A light sweep while this card's artwork is still arriving.
#[derive(Component)]
pub struct GridLoading(usize);
#[derive(Component)]
pub struct GridDim(usize);

#[derive(Resource, Default)]
pub struct GridAnim {
    /// First fully shown row, and the smoothed scroll (canvas px).
    first_row: usize,
    scroll: f32,
    /// How much the grid view is showing (0..1).
    shown: f32,
    /// The selection the header is showing, and its slide-in progress.
    key: Option<(usize, u64)>,
    t: f32,
}

/// Room above the first row inside the clipped layer, so a selected card's pop and glow
/// aren't cut off.
const PAD: f32 = 60.0;

pub fn spawn(commands: &mut Commands) {
    commands.spawn((
        GridShade,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::NONE),
        GlobalZIndex(4),
        Pickable::IGNORE,
    ));
    commands
        .spawn((
            GridHeader,
            Node {
                position_type: PositionType::Absolute,
                left: px(SIDE),
                right: px(SIDE),
                top: px(100),
                // Keeps clear of the top bar above and, below, of the first card row —
                // the selected card lifts 10px and its glow reaches a little further.
                height: px(GRID_TOP - 100.0 - 22.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexEnd,
                display: Display::None,
                ..default()
            },
            UiTransform::default(),
            GlobalZIndex(6),
            Pickable::IGNORE,
        ))
        .with_children(|h| {
            h.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(4), flex_grow: 1.0, ..default() }, Pickable::IGNORE))
                .with_children(|c| {
                    // One line only: a wrapped title would climb into the tab bar above.
                    c.spawn((GridText::Title, Text::new(""), TextLayout::no_wrap(), theme::font(32.0, FontWeight::SEMIBOLD), TextColor(WHITE), Pickable::IGNORE));
                    c.spawn((
                        GridText::Meta,
                        Text::new(""),
                        theme::font(15.0, FontWeight::NORMAL),
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.7)),
                        Pickable::IGNORE,
                    ));
                });
            h.spawn((
                GridText::Count,
                Text::new(""),
                theme::numeric_font(15.0, FontWeight::NORMAL),
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
                Pickable::IGNORE,
            ));
        });
    commands.spawn((
        GridLayer,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(GRID_TOP - PAD),
            bottom: px(GRID_BOTTOM),
            overflow: Overflow::clip(),
            display: Display::None,
            ..default()
        },
        UiTransform::default(),
        GlobalZIndex(6),
        Pickable::IGNORE,
    ));
}

/// Canvas size (the UI is laid out on a 900-tall canvas).
fn canvas(windows: &Query<&Window, With<PrimaryWindow>>, scale: &UiScale) -> Vec2 {
    windows
        .single()
        .map(|w| Vec2::new(w.width(), w.height()) / scale.0.max(0.01))
        .unwrap_or(Vec2::new(1600.0, 900.0))
}

fn columns(width: f32) -> usize {
    (((width - 2.0 * SIDE + GRID_GAP) / (GRID_W + GRID_GAP)).floor() as usize).max(1)
}

fn visible_rows(height: f32) -> usize {
    (((height - GRID_TOP - GRID_BOTTOM + GRID_GAP) / (GRID_H + GRID_GAP)).floor() as usize).max(1)
}

/// Spawns one card per game when the grid's content changes.
pub fn sync_grid(
    mut commands: Commands,
    dash: Res<Dash>,
    lib: Res<Library>,
    tex: Res<Textures>,
    mut shimmer_mats: ResMut<Assets<ShimmerMaterial>>,
    layer: Query<Entity, With<GridLayer>>,
    mut anim: ResMut<GridAnim>,
    mut last: Local<Option<(usize, u64)>>,
) {
    let Ok(layer) = layer.single() else { return };
    let showing = dash.in_grid();
    let key = (dash.current, dash.content_gen);
    let blade_changed = last.is_some_and(|(blade, _)| blade != dash.current);
    let entering = showing && (last.is_none() || blade_changed);
    if !showing || (*last == Some(key) && !entering) {
        return;
    }
    *last = Some(key);
    commands.entity(layer).despawn_children();
    let blade = dash.blade();
    let sel = dash.sel[dash.current];
    if blade_changed {
        anim.first_row = sel / dash.grid_cols.max(1);
        anim.scroll = anim.first_row as f32 * (GRID_H + GRID_GAP);
    }
    let first = anim.first_row * dash.grid_cols.max(1);
    commands.entity(layer).with_children(|g| {
        for (i, e) in blade.entries.iter().enumerate() {
            let game = game_of(&lib, e);
            // Cards ripple in from the top-left when the tab opens.
            let appear = if entering { -0.012 * i.saturating_sub(first) as f32 } else { 1.0 };
            let fb_tint = game
                .map(|g| fx::game_accent(g, &tex).unwrap_or_else(|| super::view::system_color(g.system)))
                .unwrap_or(blade.color);
            g.spawn((
                GridTile { idx: i, sel: if i == sel { 1.0 } else { 0.0 }, vel: 0.0, appear },
                Hovered::default(),
                Node {
                    position_type: PositionType::Absolute,
                    width: px(GRID_W),
                    height: px(GRID_H),
                    border_radius: BorderRadius::all(px(CARD_R * 0.8)),
                    // A hairline of light along the top edge, like the bevel of a glass card.
                    border: UiRect::top(px(1)),
                    overflow: Overflow::clip(),
                    display: Display::None,
                    ..default()
                },
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.22)),
                Outline { width: px(2.5), offset: px(0), color: Color::NONE },
                BoxShadow(vec![shadow(Color::NONE, 0.0, 18.0, 0.0), shadow(Color::srgba(0.0, 0.0, 0.0, 0.55), 10.0, 22.0, 0.0)]),
                // See-through behind the art, so the backdrop glows through the cards.
                BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.35)),
                UiTransform::default(),
                GlobalZIndex(7),
            ))
            .with_children(|t| {
                // Translucent tint so the backdrop — or the generated cover — shows
                // through fallback cards too. The game's own colour when its hero has
                // one, else the blade's.
                let base = fb_tint;
                t.spawn((
                    GridFallback(i),
                    Node {
                        position_type: PositionType::Absolute,
                        width: percent(100),
                        height: percent(100),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::FlexEnd,
                        padding: UiRect::all(px(10)),
                        border_radius: BorderRadius::all(px(CARD_R * 0.8)),
                        ..default()
                    },
                    BackgroundColor(theme::alpha(darken(base, 0.7), 0.40)),
                    Pickable::IGNORE,
                ))
                .with_children(|f| {
                    f.spawn((Text::new(e.title.clone()), theme::font(13.0, FontWeight::SEMIBOLD), TextColor(WHITE), Pickable::IGNORE));
                });
                t.spawn((
                    GridImage(i),
                    ImageNode::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        width: percent(100),
                        height: percent(100),
                        border_radius: BorderRadius::all(px(CARD_R * 0.8)),
                        display: Display::None,
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
                if let Some(g) = game {
                    // A light sweep while the cover is still on its way.
                    t.spawn((
                        GridLoading(i),
                        MaterialNode(shimmer_mats.add(ShimmerMaterial {
                            params: Vec4::new((fx::hash(&g.id) % 4096) as f32 / 4096.0, 0.0, 0.0, 0.0),
                        })),
                        Node {
                            position_type: PositionType::Absolute,
                            width: percent(100),
                            height: percent(100),
                            border_radius: BorderRadius::all(px(CARD_R * 0.8)),
                            display: Display::None,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                }
                // Glass sheen across the top, shade toward the bottom: reads as a raised card.
                t.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: percent(100),
                        height: percent(100),
                        border_radius: BorderRadius::all(px(CARD_R * 0.8)),
                        ..default()
                    },
                    BackgroundGradient::from(LinearGradient {
                        angle: LinearGradient::TO_BOTTOM,
                        stops: vec![
                            ColorStop::new(Color::srgba(1.0, 1.0, 1.0, 0.16), percent(0)),
                            ColorStop::new(Color::srgba(1.0, 1.0, 1.0, 0.0), percent(38)),
                            ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.0), percent(70)),
                            ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.28), percent(100)),
                        ],
                        ..default()
                    }),
                    Pickable::IGNORE,
                ));
                t.spawn((
                    GridDim(i),
                    Node {
                        position_type: PositionType::Absolute,
                        width: percent(100),
                        height: percent(100),
                        border_radius: BorderRadius::all(px(CARD_R * 0.8)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    Pickable::IGNORE,
                ));
            });
        }
    });
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn animate_grid(
    mut dash: ResMut<Dash>,
    lib: Res<Library>,
    mut tex: ResMut<Textures>,
    time: Res<Time>,
    mut fx: ResMut<Fx>,
    scale: Res<UiScale>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut anim: ResMut<GridAnim>,
    mut tiles: Query<(&mut GridTile, &mut Node, &mut Outline, &mut BoxShadow, &mut UiTransform, &mut GlobalZIndex)>,
    mut images: Query<(&GridImage, &mut ImageNode, &mut Node), Without<GridTile>>,
    mut fallbacks: Query<(&GridFallback, &mut Node), (Without<GridTile>, Without<GridImage>)>,
    mut shimmers: Query<
        (&GridLoading, &mut Node),
        (Without<GridTile>, Without<GridImage>, Without<GridFallback>, Without<GridHeader>),
    >,
    mut dims: Query<(&GridDim, &mut BackgroundColor)>,
    mut texts: Query<(&GridText, &mut Text, &mut TextColor)>,
    mut header: Query<
        (&mut Node, &mut UiTransform),
        (
            With<GridHeader>,
            Without<GridTile>,
            Without<GridImage>,
            Without<GridFallback>,
            Without<GridLoading>,
        ),
    >,
) {
    let dt = time.delta_secs().min(1.0 / 20.0);
    let size = canvas(&windows, &scale);
    let cols = columns(size.x);
    if dash.grid_cols != cols {
        dash.grid_cols = cols;
    }
    if !dash.in_grid() {
        anim.shown = 0.0;
        return;
    }
    anim.shown = (anim.shown + dt * 4.0).min(1.0);
    let rows = visible_rows(size.y);
    let sel = dash.sel[dash.current];
    let n = dash.blade().entries.len();

    // Keep the selected row in view, scrolling as little as possible.
    let row = sel / cols;
    if row < anim.first_row {
        anim.first_row = row;
    } else if row >= anim.first_row + rows {
        anim.first_row = row + 1 - rows;
    }
    let last_first = (n.saturating_sub(1) / cols + 1).saturating_sub(rows);
    anim.first_row = anim.first_row.min(last_first);
    let target = anim.first_row as f32 * (GRID_H + GRID_GAP);
    anim.scroll = approach(anim.scroll, target, 12.0, dt);

    let width = cols as f32 * GRID_W + (cols - 1) as f32 * GRID_GAP;
    let x0 = ((size.x - width) / 2.0).round();
    // The header lines up with the grid's edges, and its text slides in from the left
    // whenever the selection moves — the info block's gesture, at grid scale.
    if let Ok((mut h, mut tf)) = header.single_mut() {
        set_px(&mut h, |n| &mut n.left, x0);
        set_px(&mut h, |n| &mut n.right, x0);
        let key = (sel, dash.content_gen);
        if anim.key != Some(key) {
            anim.key = Some(key);
            anim.t = 0.0;
        }
        anim.t = (anim.t + dt * 4.0).min(1.0);
        let e = ease(anim.t);
        let want = UiTransform { translation: Val2::px(-12.0 * (1.0 - e), 0.0), ..default() };
        if *tf != want {
            *tf = want;
        }
    }
    let accent = fx.accent_color();
    let ring = Color::from(accent.to_srgba().mix(&Srgba::WHITE, 0.35));
    let breathe = 0.85 + 0.15 * (time.elapsed_secs() * 2.4).sin();
    let launch = fx.launch_env();
    let press = fx.press_env();
    // Distance in cards from the selection: neighbours swell a touch, far cards recede.
    let (sc0, sr0) = ((sel % cols) as f32, (sel / cols) as f32);
    let dist = |i: usize| (((i % cols) as f32 - sc0).powi(2) + ((i / cols) as f32 - sr0).powi(2)).sqrt();
    // The row above the view only shows while it scrolls away (its bottom edge would
    // otherwise peek into the space kept for the first row's shadows).
    let scrolling = (anim.scroll - target).abs() > 1.0;
    let in_view = |i: usize| {
        let r = i / cols;
        (r >= anim.first_row || (scrolling && r + 1 >= anim.first_row)) && r <= anim.first_row + rows
    };

    let mut state: Vec<(usize, f32, f32)> = Vec::with_capacity(n);
    for (mut t, mut node, mut outline, mut shadows, mut tf, mut z) in &mut tiles {
        let want = if t.idx == sel { 1.0 } else { 0.0 };
        for _ in 0..2 {
            let h = dt * 0.5;
            let acc = 260.0 * (want - t.sel) - 17.0 * t.vel;
            t.vel += acc * h;
            t.sel += t.vel * h;
        }
        if (t.sel - want).abs() < 1e-3 && t.vel.abs() < 1e-3 {
            t.sel = want;
            t.vel = 0.0;
        }
        t.appear = (t.appear + dt * 4.0).min(1.0);
        state.push((t.idx, t.sel, t.appear));

        let (c, r) = (t.idx % cols, t.idx / cols);
        let x = x0 + c as f32 * (GRID_W + GRID_GAP);
        let y = PAD + r as f32 * (GRID_H + GRID_GAP) - anim.scroll;
        set_display(&mut node, in_view(t.idx));
        set_px(&mut node, |n| &mut n.left, x.round());
        set_px(&mut node, |n| &mut n.top, y.round());
        let sc = t.sel.clamp(0.0, 1.0);
        let col = super::theme::alpha(ring, sc);
        if outline.color != col {
            outline.color = col;
        }
        let is_sel = t.idx == sel;
        let flare = if is_sel { launch } else { 0.0 };
        let dip = if is_sel { press } else { 0.0 };
        let glow = super::theme::alpha(accent, (0.6 * sc * breathe + 0.4 * flare + 0.3 * dip).min(1.0));
        if shadows.0[0].color != glow {
            shadows.0[0].color = glow;
        }
        // The selected card lifts off: its drop shadow grows and falls further away.
        let drop = Color::srgba(0.0, 0.0, 0.0, 0.5 + 0.3 * sc);
        if shadows.0[1].color != drop {
            shadows.0[1].color = drop;
            shadows.0[1].y_offset = px(10.0 + 18.0 * sc);
            shadows.0[1].blur_radius = px(22.0 + 24.0 * sc);
        }
        let e = ease(t.appear);
        // Only the selected card moves: it lifts straight up on its spring (overshooting
        // a touch, then settling back down) and dips under an A-press. Every other card
        // stays perfectly still.
        let want_tf = UiTransform {
            translation: Val2::px(0.0, (1.0 - e) * 24.0 - 10.0 * sc - 8.0 * dip),
            scale: Vec2::splat(1.0 - 0.04 * dip),
            ..default()
        };
        if *tf != want_tf {
            *tf = want_tf;
        }
        // The selected card draws over its neighbours.
        let want_z = GlobalZIndex(if t.idx == sel { 8 } else { 7 });
        if *z != want_z {
            *z = want_z;
        }
    }

    // A launch bursts from the selected card: keep its spot (canvas → uv).
    let (c, r) = (sel % cols, sel / cols);
    let ox = x0 + c as f32 * (GRID_W + GRID_GAP) + GRID_W * 0.5;
    let oy = GRID_TOP + r as f32 * (GRID_H + GRID_GAP) - anim.scroll + GRID_H * 0.5;
    fx.origin = Vec2::new(ox / size.x, oy / size.y);

    let mut missing = Vec::new();
    let mut loading = Vec::new();
    for (gi, mut img, mut node) in &mut images {
        if !in_view(gi.0) {
            continue;
        }
        let Some(game) = dash.blade().entries.get(gi.0).and_then(|e| game_of(&lib, e)) else { continue };
        match tex.get(&game.id, ArtKind::Cover) {
            Some((h, _)) => {
                if img.image != h {
                    img.image = h;
                }
                // Glassy: the art is a little see-through until selected.
                let s = state.iter().find(|x| x.0 == gi.0).map(|x| x.1.clamp(0.0, 1.0)).unwrap_or(0.0);
                let c = Color::srgba(1.0, 1.0, 1.0, 0.85 + 0.15 * s);
                if img.color != c {
                    img.color = c;
                }
                set_display(&mut node, true);
            }
            None => {
                set_display(&mut node, false);
                if tex.is_missing(&game.id, ArtKind::Cover) {
                    missing.push(gi.0);
                    // No art anywhere: the generated cover, tinted like real art — the
                    // game's own colour when its hero has one.
                    let fb_accent = fx::game_accent(game, &tex).unwrap_or_else(|| super::view::system_color(game.system));
                    if let Some(fb) = tex.fallback(&game.id, fb_accent) {
                        if img.image != fb {
                            img.image = fb;
                        }
                        let s = state.iter().find(|x| x.0 == gi.0).map(|x| x.1.clamp(0.0, 1.0)).unwrap_or(0.0);
                        let c = Color::srgba(1.0, 1.0, 1.0, 0.85 + 0.15 * s);
                        if img.color != c {
                            img.color = c;
                        }
                        set_display(&mut node, true);
                    }
                } else {
                    loading.push(gi.0);
                }
            }
        }
    }
    for (f, mut node) in &mut fallbacks {
        set_display(&mut node, missing.contains(&f.0) || loading.contains(&f.0));
    }
    for (s, mut node) in &mut shimmers {
        set_display(&mut node, loading.contains(&s.0));
    }
    for (d, mut bg) in &mut dims {
        let (s, appear) = state.iter().find(|x| x.0 == d.0).map(|x| (x.1.clamp(0.0, 1.0), ease(x.2))).unwrap_or((0.0, 1.0));
        // Far from the selection, cards step back a touch — but only a touch, so the
        // backdrop still reads through every card.
        let a = ((0.10 + 0.16 * (dist(d.0) / 4.0).min(1.0)) * (1.0 - s)).max(1.0 - appear);
        set_color(&mut bg, BackgroundColor(Color::srgba(0.01, 0.015, 0.025, a)));
    }

    // Header: the selected game.
    let entry = dash.selected().cloned();
    let game = entry.as_ref().and_then(|e| game_of(&lib, e));
    let title = entry.as_ref().map(|e| e.title.clone()).unwrap_or_default();
    let meta = game
        .map(|g| {
            // The Controller grid leads with the support verdict.
            if matches!(dash.blade().key, BladeKey::Controller) {
                format!("{}  •  {}", g.controller_label(), game_meta(g))
            } else {
                game_meta(g)
            }
        })
        .unwrap_or_default();
    let count = if n == 0 { String::new() } else { format!("{} / {}", sel + 1, n) };
    let a = anim.shown * ease(anim.t);
    for (k, mut text, mut color) in &mut texts {
        let (s, alpha_max) = match k {
            GridText::Title => (&title, 1.0),
            GridText::Meta => (&meta, 0.7),
            GridText::Count => (&count, 0.55),
        };
        set_text(&mut text, s);
        set_color(&mut color, TextColor(Color::srgba(1.0, 1.0, 1.0, alpha_max * a)));
    }
}

/// Shows the grid or the shelf view, whichever the current tab uses.
#[allow(clippy::type_complexity)]
pub fn toggle_views(
    dash: Res<Dash>,
    anim: Res<GridAnim>,
    fx: Res<Fx>,
    mut grid: Query<&mut Node, Or<(With<GridLayer>, With<GridHeader>)>>,
    mut shade: Query<(&mut Node, &mut BackgroundColor), (With<GridShade>, Without<GridLayer>, Without<GridHeader>)>,
    mut shelf: Query<
        &mut Node,
        (Or<(With<InfoBlock>, With<ShelfLayer>, With<ShelfCount>)>, Without<GridLayer>, Without<GridHeader>, Without<GridShade>),
    >,
) {
    let on = dash.in_grid();
    for mut n in &mut grid {
        set_display(&mut n, on);
    }
    for mut n in &mut shelf {
        set_display(&mut n, !on);
    }
    if let Ok((mut n, mut bg)) = shade.single_mut() {
        set_display(&mut n, on);
        // Ambient racks the cards away, so the dim meant to keep them readable goes too.
        set_color(&mut bg, BackgroundColor(Color::srgba(0.01, 0.015, 0.025, 0.12 * anim.shown * (1.0 - fx.ambient))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_16_9_screen_shows_the_whole_small_library() {
        // 1600x900 canvas: 12 columns x 4 rows.
        assert_eq!(columns(1600.0), 12);
        assert_eq!(visible_rows(900.0), 4);
        // Ultrawide gets more columns.
        assert!(columns(2100.0) > 15);
    }
}
