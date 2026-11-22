//! The grid ↔ shelf morph. Switching between a grid tab (All, Wishlist, Controller)
//! and a shelf tab carries the selection's cover across layouts: it detaches from its
//! cell in the view you left and flies to its slot in the one you entered — one
//! continuous move while the rest of the new view racks in around the reserved slot,
//! whose own tile waits hidden until the cover lands and swaps in.

use bevy::prelude::*;
use bevy::ui::UiScale;
use bevy::window::PrimaryWindow;

use super::fx::{self, Fx};
use super::grid::{self, GridAnim};
use super::state::{Action, BladeKey, Dash, Library, Mode};
use super::textures::Textures;
use super::theme::*;
use super::view::{self, ShelfAnim, ease, game_of, set_color, set_px, shadow};

/// The flying card's own layer: above every tile (the shelf sits at 6, the grid at
/// 7–8), below the chrome (prompts, overlays) — a tile moving through the room, not a
/// HUD element. It is never clipped, so the card can cross view boundaries freely.
#[derive(Component)]
pub struct FlightLayer;

#[derive(Component)]
pub struct FlightCard;

#[derive(Component)]
pub struct FlightImage;

#[derive(Component)]
pub struct FlightDim;

/// Progress rate: the flight takes half a second, cubic-eased out like the rest of
/// the motion, landing just as the rack-in settles.
const SPEED: f32 = 2.0;

/// A called-off card dissolves over this many seconds.
const FADE: f32 = 0.12;

/// Where a tile sits on the canvas: centre and size, canvas px.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Rect {
    c: Vec2,
    s: Vec2,
}

impl Rect {
    fn new(cx: f32, cy: f32, w: f32, h: f32) -> Self {
        Self { c: Vec2::new(cx, cy), s: Vec2::new(w, h) }
    }
}

/// How a tile looks in either layout, selected or not — everything the flying card
/// has to wear so each end of the flight is a handoff, not a pop.
#[derive(Clone, Copy, Default)]
struct TileStyle {
    w: f32,
    h: f32,
    radius: f32,
    /// Ring width and alpha (an unselected tile has no ring).
    ring: (f32, f32),
    /// Card background alpha: grid cards are glassy, shelf cards near-opaque.
    bg: f32,
    img: f32,
    dim: f32,
    /// Glow strength (in the game's colour) and blur.
    glow: (f32, f32),
    /// Drop shadow: alpha, y offset, blur.
    drop: (f32, f32, f32),
    /// The grid card's hairline top bevel.
    border: f32,
}

fn tile_style(grid: bool, sel: bool) -> TileStyle {
    let s = if sel { 1.0 } else { 0.0 };
    if grid {
        TileStyle {
            w: GRID_W,
            h: GRID_H,
            radius: CARD_R * 0.8,
            ring: (2.5, s),
            bg: 0.35,
            img: 0.85 + 0.15 * s,
            dim: 0.26 * (1.0 - s),
            glow: (0.6 * s, 18.0),
            drop: (0.5 + 0.3 * s, 10.0, 22.0),
            border: 0.22,
        }
    } else {
        TileStyle {
            w: TILE_W + (TILE_SEL_W - TILE_W) * s,
            h: TILE_H + (TILE_SEL_H - TILE_H) * s,
            radius: CARD_R,
            ring: (3.0, s),
            bg: 0.9,
            img: 1.0,
            dim: 0.55 * (1.0 - s),
            glow: (0.55 * s, 28.0),
            drop: (0.6 + 0.3 * s, 16.0, 34.0),
            border: 0.0,
        }
    }
}

fn lerp_style(a: TileStyle, b: TileStyle, t: f32) -> TileStyle {
    let l = |x: f32, y: f32| x + (y - x) * t;
    TileStyle {
        w: l(a.w, b.w),
        h: l(a.h, b.h),
        radius: l(a.radius, b.radius),
        ring: (l(a.ring.0, b.ring.0), l(a.ring.1, b.ring.1)),
        bg: l(a.bg, b.bg),
        img: l(a.img, b.img),
        dim: l(a.dim, b.dim),
        glow: (l(a.glow.0, b.glow.0), l(a.glow.1, b.glow.1)),
        drop: (l(a.drop.0, b.drop.0), l(a.drop.1, b.drop.1), l(a.drop.2, b.drop.2)),
        border: l(a.border, b.border),
    }
}

#[derive(Resource, Default)]
pub struct Flight {
    /// The game in flight, by entry action; `None` while grounded.
    action: Option<Action>,
    /// The blade the card is landing in, and its tile's index there.
    dest: usize,
    idx: usize,
    into_grid: bool,
    from: Rect,
    /// Where the card is right now — a redirect picks up from here.
    last: Rect,
    /// The look it left with. The look it lands in is read live, so a selection that
    /// moves mid-flight finds the card still on target.
    src: TileStyle,
    t: f32,
    /// Called off (a launch, an overlay, a blade it can't land on): dissolve into the
    /// rack-in instead of landing.
    fading: bool,
    fade: f32,
    cover: Option<Handle<Image>>,
    /// Scripted 'F': hold the card mid-air so a screenshot can catch the morph.
    pub freeze: bool,
    /// The library generation the flight was bound to.
    gens: (u64, u64),
}

impl Flight {
    fn active(&self) -> bool {
        self.action.is_some()
    }

    /// The tile at `idx` on `blade` waits hidden while the card flies at its slot.
    pub fn hiding(&self, blade: usize, idx: usize) -> bool {
        !self.fading && self.action.is_some() && blade == self.dest && idx == self.idx
    }
}

/// Where tile `idx` sits in a `cols`-column grid scrolled by `scroll` (canvas px). The
/// selected tile rides 10 px up, the grid's own selected-card lift.
fn grid_slot(size_x: f32, cols: usize, scroll: f32, sel: usize, idx: usize) -> Rect {
    let cols = cols.max(1);
    let step = GRID_W + GRID_GAP;
    let width = cols as f32 * GRID_W + (cols - 1) as f32 * GRID_GAP;
    let x0 = ((size_x - width) / 2.0).round();
    let (c, r) = (idx % cols, idx / cols);
    Rect::new(
        x0 + c as f32 * step + GRID_W * 0.5,
        GRID_TOP + r as f32 * step - scroll + GRID_H * 0.5 - if idx == sel { 10.0 } else { 0.0 },
        GRID_W,
        GRID_H,
    )
}

/// Where tile `idx` sits on a shelf of `n` tiles with selection `sel`, scrolled by
/// `scroll` — the same left-to-right walk `animate_shelf` lays the row out with.
fn shelf_slot(n: usize, sel: usize, scroll: f32, idx: usize) -> Rect {
    let mut x = SIDE - scroll;
    let mut w = TILE_W;
    for i in 0..n {
        w = if i == sel { TILE_SEL_W } else { TILE_W };
        if i == idx {
            break;
        }
        x += w + TILE_GAP;
    }
    let h = if idx == sel { TILE_SEL_H } else { TILE_H };
    Rect::new(x + w * 0.5, CANVAS_H - SHELF_BOTTOM - h * 0.5, w, h)
}

fn blade_grid(dash: &Dash, blade: usize) -> bool {
    dash.blades
        .get(blade)
        .is_some_and(|b| matches!(b.key, BladeKey::All | BladeKey::Wishlist | BladeKey::Controller))
}

/// Spawns the flight layer at startup.
pub fn spawn(mut commands: Commands) {
    commands.spawn((
        FlightLayer,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: percent(100),
            height: percent(100),
            ..default()
        },
        GlobalZIndex(9),
        Pickable::IGNORE,
    ));
}

/// Watches blade switches: a grid ↔ shelf move sends the new selection's cover flying
/// from the layout you leave to the slot it lands in. Runs right after navigation and
/// before the views respawn, while the old layout's scroll is still live.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn start_flight(
    mut commands: Commands,
    dash: Res<Dash>,
    lib: Res<Library>,
    mut tex: ResMut<Textures>,
    grid_anim: Res<GridAnim>,
    shelf_anim: Res<ShelfAnim>,
    scale: Res<UiScale>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layer: Query<Entity, With<FlightLayer>>,
    cards: Query<Entity, With<FlightCard>>,
    mut flight: ResMut<Flight>,
    mut last: Local<Option<(usize, u64, u64)>>,
) {
    // A card that reached its slot last frame hands it back to its tile now, before
    // the views lay out — the same frame, so there is never a frame with neither.
    if flight.active() && !flight.fading && !flight.freeze && flight.t >= 1.0 {
        if let Ok(card) = cards.single() {
            commands.entity(card).despawn();
        }
        *flight = Flight::default();
    }

    let key = (dash.current, dash.layout_gen, dash.content_gen);
    let prev = *last;
    *last = Some(key);
    let Some((prev_blade, lg, cg)) = prev else { return };
    let clean = prev_blade != dash.current && (lg, cg) == (dash.layout_gen, dash.content_gen);

    if flight.active() {
        if prev_blade == dash.current {
            return;
        }
        // The blade moved under a mid-air card. Landing on the same game carries it
        // on from wherever it is; anything else dissolves it into the rack-in.
        // Never two cards at once.
        if clean
            && dash.in_grid() != blade_grid(&dash, prev_blade)
            && dash.selected().is_some_and(|e| Some(e.action) == flight.action)
        {
            let is_sel = flight.idx == dash.sel[flight.dest];
            flight.src = lerp_style(flight.src, tile_style(flight.into_grid, is_sel), ease(flight.t));
            flight.dest = dash.current;
            flight.idx = dash.sel[dash.current];
            flight.into_grid = dash.in_grid();
            flight.from = flight.last;
            flight.gens = (dash.layout_gen, dash.content_gen);
            flight.fading = false;
            flight.fade = 1.0;
            return;
        }
        flight.fading = true;
        return;
    }

    if !clean {
        return;
    }
    let from_grid = blade_grid(&dash, prev_blade);
    let into_grid = dash.in_grid();
    if from_grid == into_grid {
        // The morph is for layout swaps only; shelf→shelf and grid→grid keep their
        // own entrances.
        return;
    }

    // The game the new tab lands on — but only if it's on screen in the view we just
    // left; a cover detaching from an empty spot would read as an arrival, not a move.
    let Some(entry) = dash.selected() else { return };
    let old = &dash.blades[prev_blade];
    let Some(idx) = old.entries.iter().position(|e| e.action == entry.action) else { return };
    let size = grid::canvas(&windows, &scale);
    let sel_old = dash.sel[prev_blade];
    let from = if from_grid {
        let (cols, rows) = (grid::columns(size.x), grid::visible_rows(size.y));
        let r = idx / cols;
        if r < grid_anim.first_row || r > grid_anim.first_row + rows {
            return;
        }
        grid_slot(size.x, cols, grid_anim.scroll, sel_old, idx)
    } else {
        let rect = shelf_slot(old.entries.len(), sel_old, shelf_anim.scroll, idx);
        if rect.c.x - rect.s.x * 0.5 >= size.x || rect.c.x + rect.s.x * 0.5 <= 0.0 {
            return;
        }
        rect
    };

    // The card carries the tile's own art: its cover, or the generated one.
    let Some(game) = game_of(&lib, entry) else { return };
    let fb_accent = fx::game_accent(game, &tex).unwrap_or_else(|| view::system_color(game.system));
    let cover = tex
        .get(&game.id, crate::art::ArtKind::Cover)
        .map(|(h, _)| h)
        .or_else(|| tex.fallback(&game.id, fb_accent));
    let Some(cover) = cover else { return };

    flight.action = Some(entry.action);
    flight.dest = dash.current;
    flight.idx = dash.sel[dash.current];
    flight.into_grid = into_grid;
    flight.from = from;
    flight.last = from;
    flight.src = tile_style(from_grid, idx == sel_old);
    flight.t = 0.0;
    flight.fading = false;
    flight.fade = 1.0;
    flight.cover = Some(cover.clone());
    flight.gens = (dash.layout_gen, dash.content_gen);

    let Ok(layer) = layer.single() else { return };
    commands.entity(layer).despawn_children();
    commands.entity(layer).with_children(|l| {
        let radius = if from_grid { CARD_R * 0.8 } else { CARD_R };
        l.spawn((
            FlightCard,
            Node {
                position_type: PositionType::Absolute,
                left: px(from.c.x - from.s.x * 0.5),
                top: px(from.c.y - from.s.y * 0.5),
                width: px(from.s.x),
                height: px(from.s.y),
                border_radius: BorderRadius::all(px(radius)),
                border: UiRect::top(px(1)),
                overflow: Overflow::clip(),
                ..default()
            },
            Outline { width: px(2.5), offset: px(0), color: Color::NONE },
            BoxShadow(vec![
                shadow(Color::NONE, 0.0, 18.0, 0.0),
                shadow(Color::srgba(0.0, 0.0, 0.0, 0.6), 10.0, 22.0, 0.0),
            ]),
            BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.35)),
            BorderColor::all(Color::NONE),
            UiTransform::default(),
            // Transformed children escape the parent's stacking; pin them explicitly.
            GlobalZIndex(9),
            Pickable::IGNORE,
        ))
        .with_children(|card| {
            card.spawn((
                FlightImage,
                ImageNode { image: cover, ..default() },
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    border_radius: BorderRadius::all(px(radius)),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            card.spawn((
                FlightDim,
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    border_radius: BorderRadius::all(px(radius)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                Pickable::IGNORE,
            ));
        });
    });
}

/// Flies the card. The landing slot is read live — a selection that moves mid-flight
/// simply finds the card still on target — and the style interpolates into exactly
/// what the waiting tile wears, so landing is a swap.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn animate_flight(
    mut commands: Commands,
    dash: Res<Dash>,
    time: Res<Time>,
    fx: Res<Fx>,
    mut flight: ResMut<Flight>,
    grid_anim: Res<GridAnim>,
    shelf_anim: Res<ShelfAnim>,
    scale: Res<UiScale>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cards: Query<
        (
            Entity,
            &FlightCard,
            &mut Node,
            &mut Outline,
            &mut BoxShadow,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut UiTransform,
        ),
    >,
    mut images: Query<(&FlightImage, &mut ImageNode, &mut Node), Without<FlightCard>>,
    mut dims: Query<
        (&FlightDim, &mut BackgroundColor, &mut Node),
        (Without<FlightCard>, Without<FlightImage>),
    >,
) {
    if !flight.active() {
        return;
    }
    let dt = time.delta_secs().min(1.0 / 20.0);

    // A launch, an overlay, ambient or a library refresh takes over the screen: the
    // card dissolves where it is rather than landing into someone else's moment.
    if (dash.layout_gen, dash.content_gen) != flight.gens
        || dash.mode != Mode::Browse
        || fx.launch.is_some()
        || fx.ambient > 0.5
        || flight.dest != dash.current
    {
        flight.fading = true;
    }

    if flight.fading {
        flight.fade -= dt / FADE;
        if flight.fade <= 0.0 {
            if let Ok((card, ..)) = cards.single() {
                commands.entity(card).despawn();
            }
            let freeze = flight.freeze;
            *flight = Flight { freeze, ..default() };
            return;
        }
    } else if flight.freeze {
        // Scripted screenshots only: hold the card mid-air. 0.21 is the eased
        // halfway point — with cubic ease-out, t = 0.5 is already 87.5% of the way.
        flight.t = 0.21;
    } else {
        // Landing at exactly 1.0 is grounded by `start_flight` next frame, so the
        // tile underneath takes over the frame the card disappears — no gap.
        flight.t = (flight.t + dt * SPEED).min(1.0);
    }

    let Ok((_, _, mut node, mut outline, mut shadows, mut bg, mut border, mut tf)) = cards.single_mut() else {
        // No card to fly (only possible if spawning failed): clear the flight rather
        // than leave a tile hidden forever.
        *flight = Flight::default();
        return;
    };
    let size = grid::canvas(&windows, &scale);
    let sel = dash.sel[dash.current];
    let to = if flight.into_grid {
        grid_slot(size.x, grid::columns(size.x), grid_anim.scroll, sel, flight.idx)
    } else {
        shelf_slot(dash.blade().entries.len(), sel, shelf_anim.scroll, flight.idx)
    };
    let e = ease(flight.t);
    let fade = flight.fade.clamp(0.0, 1.0);
    let st = lerp_style(flight.src, tile_style(flight.into_grid, flight.idx == sel), e);

    // The path bows slightly against gravity, and the card leans into the travel —
    // both gone by the time it lands.
    let dir = to.c - flight.from.c;
    let mut n = Vec2::new(dir.y, -dir.x);
    if n.y > 0.0 {
        n = -n;
    }
    let bow = n.normalize_or_zero() * (dir.length() * 0.08).min(26.0) * (std::f32::consts::PI * e).sin();
    let c = flight.from.c.lerp(to.c, e) + bow;
    flight.last = Rect { c, s: Vec2::new(st.w, st.h) };

    set_px(&mut node, |n| &mut n.left, (c.x - st.w * 0.5).round());
    set_px(&mut node, |n| &mut n.top, (c.y - st.h * 0.5).round());
    set_px(&mut node, |n| &mut n.width, st.w.round());
    set_px(&mut node, |n| &mut n.height, st.h.round());
    let radius = BorderRadius::all(px(st.radius));
    if node.border_radius != radius {
        node.border_radius = radius;
    }

    // Ring, glow and drop exactly as the tile it lands on wears them.
    let accent = fx.accent_color();
    let ring = alpha(Color::from(accent.to_srgba().mix(&Srgba::WHITE, 0.35)), st.ring.1 * fade);
    if outline.color != ring {
        outline.color = ring;
        if outline.width != px(st.ring.0) {
            outline.width = px(st.ring.0);
        }
    }
    let breathe = 0.85 + 0.15 * (time.elapsed_secs() * 2.4).sin();
    let glow = alpha(accent, (st.glow.0 * breathe * fade).min(1.0));
    if shadows.0[0].color != glow {
        shadows.0[0].color = glow;
        shadows.0[0].blur_radius = px(st.glow.1);
    }
    let drop = Color::srgba(0.0, 0.0, 0.0, st.drop.0 * fade);
    if shadows.0[1].color != drop {
        shadows.0[1].color = drop;
        shadows.0[1].y_offset = px(st.drop.1);
        shadows.0[1].blur_radius = px(st.drop.2);
    }
    set_color(&mut bg, BackgroundColor(Color::srgba(0.05, 0.06, 0.08, st.bg * fade)));
    set_color(&mut border, BorderColor::all(Color::srgba(1.0, 1.0, 1.0, st.border * fade)));
    let want_tf = UiTransform {
        rotation: Rot2::radians(0.05 * (std::f32::consts::PI * e).sin() * (dir.x / 400.0).tanh()),
        ..default()
    };
    if *tf != want_tf {
        *tf = want_tf;
    }

    for (_, mut img, mut n) in &mut images {
        let want = Color::srgba(1.0, 1.0, 1.0, st.img * fade);
        if img.color != want {
            img.color = want;
        }
        if n.border_radius != radius {
            n.border_radius = radius;
        }
    }
    for (_, mut d, mut n) in &mut dims {
        set_color(&mut d, BackgroundColor(Color::srgba(0.01, 0.015, 0.025, st.dim * fade)));
        if n.border_radius != radius {
            n.border_radius = radius;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1600-wide canvas fits 12 centred columns: x0 = 123, step = 114.
    #[test]
    fn grid_slot_matches_the_grid_layout() {
        let scroll = 2.0 * (GRID_H + GRID_GAP);
        // Tile 17: column 5, row 1; the selection is elsewhere.
        let a = grid_slot(1600.0, 12, scroll, 29, 17);
        assert_eq!(a.s, Vec2::new(GRID_W, GRID_H));
        assert!((a.c.x - (123.0 + 5.0 * 114.0 + 50.0)).abs() < 1e-4);
        assert!((a.c.y - (GRID_TOP + 114.0 + 75.0 - scroll)).abs() < 1e-4);
        // The selected tile rides 10 px up, like the grid's own selected card.
        let s = grid_slot(1600.0, 12, scroll, 17, 17);
        assert!((s.c.y - (a.c.y - 10.0)).abs() < 1e-4);
    }

    /// The shelf walk: the selected tile is wide and pushes everything after it right.
    #[test]
    fn shelf_slot_matches_the_shelf_walk() {
        let scroll = 3.0 * (TILE_W + TILE_GAP);
        let a = shelf_slot(12, 5, scroll, 5);
        assert_eq!(a.s, Vec2::new(TILE_SEL_W, TILE_SEL_H));
        assert!((a.c.x - (SIDE - scroll + 5.0 * (TILE_W + TILE_GAP) + TILE_SEL_W * 0.5)).abs() < 1e-4);
        assert!((a.c.y - (CANVAS_H - SHELF_BOTTOM - TILE_SEL_H * 0.5)).abs() < 1e-4);
        // One selected-tile step further along: centre to centre, the selected half
        // width, the gap, and the unselected half width.
        let b = shelf_slot(12, 5, scroll, 6);
        assert!((b.c.x - (a.c.x + TILE_SEL_W * 0.5 + TILE_GAP + TILE_W * 0.5)).abs() < 1e-4);
        assert_eq!(b.s, Vec2::new(TILE_W, TILE_H));
    }

    /// Each end of the flight must equal the tile it departs from / lands on.
    #[test]
    fn the_card_lands_as_the_tile_it_becomes() {
        let g = tile_style(true, true);
        assert_eq!((g.w, g.h, g.radius), (GRID_W, GRID_H, CARD_R * 0.8));
        assert_eq!(g.ring, (2.5, 1.0));
        assert_eq!(g.drop, (0.8, 10.0, 22.0));
        let s = tile_style(false, true);
        assert_eq!((s.w, s.h), (TILE_SEL_W, TILE_SEL_H));
        assert_eq!(s.ring, (3.0, 1.0));
        let u = tile_style(true, false);
        assert_eq!(u.ring.1, 0.0); // an unselected tile has no ring
        assert_eq!(u.glow.0, 0.0);
        assert!((u.drop.0 - 0.5).abs() < 1e-6);
        // The lerp hits its endpoints exactly: the handoff is a swap, not a pop.
        let (a, b) = (tile_style(true, false), tile_style(false, true));
        assert_eq!(lerp_style(a, b, 0.0).w, a.w);
        assert_eq!(lerp_style(a, b, 1.0).h, b.h);
        assert_eq!(lerp_style(a, b, 1.0).ring, b.ring);
    }

    /// The ground never shows a card pointing at a tile that isn't there.
    #[test]
    fn hiding_scopes_to_the_destination_blade() {
        let mut f = Flight::default();
        assert!(!f.hiding(0, 0));
        f.action = Some(Action::Launch(3));
        f.dest = 2;
        f.idx = 7;
        assert!(f.hiding(2, 7));
        assert!(!f.hiding(1, 7));
        assert!(!f.hiding(2, 6));
        f.fading = true;
        assert!(!f.hiding(2, 7));
    }
}
