//! The Bevy dashboard app.

mod background;
mod depth;
mod fx;
mod flight;
mod glass;
mod grid;
mod input;
mod launch;
mod particles;
mod procedural;
mod scene;
mod sound;
mod state;
mod textures;
mod theme;
mod view;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use bevy::app::AppExit;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode};
use bevy::winit::{UpdateMode, WinitSettings};

use crate::art::{self, FetchReport};
use crate::config::Config;
use crate::library::{self, Game, Launch, PlayState};
use input::{Nav, NavInput};
use sound::{Sfx, Sounds};
use state::{Action, Dash, GUIDE_ITEMS, Library, Mode};
use view::{DetailsBtn, GuideRow, KeyCap, PlayBtn, Tab, Tile};

pub const WINDOW_TITLE: &str = "gdash";

pub struct Options {
    pub windowed: bool,
    pub screenshot: Option<PathBuf>,
    /// Scripted navigation for automated screenshots: U D L R (move), Q E (category),
    /// A B Y G (buttons), K / P (keyboard / controller prompts), X (dry-run launch
    /// splash), V (force idle ambient), W (wait 4 s), F (freeze a grid↔shelf tile
    /// flight mid-air), `.` = pause.
    pub script: String,
}

#[derive(Resource)]
struct Opts {
    screenshot: Option<PathBuf>,
    script: Vec<char>,
}

#[derive(Resource, Default)]
pub struct Running(pub Option<launch::RunningGame>);

#[derive(Resource)]
struct Sys(sysinfo::System);

#[derive(Resource)]
struct Cfg(Config);

enum BgMsg {
    Library { games: Vec<Game>, wishlist: Vec<Game> },
    Art(FetchReport),
    /// ML enhancement progress: (done, total).
    Enhancing(usize, usize),
    /// A game's hero/depth were improved; reload its textures.
    Enhanced(String),
    ArtDone,
}

#[derive(Resource)]
struct Bg {
    tx: Sender<BgMsg>,
    rx: Mutex<Receiver<BgMsg>>,
    art_total: usize,
    art_done: usize,
    enhancing: Option<(usize, usize)>,
    busy: bool,
}

pub fn run(opts: Options, cfg: Config) {
    let games = library::load_cache().unwrap_or_default();
    let wishlist = library::wishlist::load(&cfg, &games);
    let first_run = games.is_empty();
    let autostart = crate::win::autostart_enabled();
    let dash = Dash::new(&games, &wishlist, autostart, cfg.sounds);

    let (tx, rx) = channel();
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(WinitSettings {
            focused_mode: UpdateMode::Continuous,
            unfocused_mode: UpdateMode::reactive_low_power(Duration::from_secs(1)),
        })
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: WINDOW_TITLE.into(),
                        mode: if opts.windowed {
                            WindowMode::Windowed
                        } else {
                            WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
                        },
                        resolution: (1600, 900).into(),
                        present_mode: PresentMode::AutoVsync,
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin { level: bevy::log::Level::WARN, ..default() })
                .set(bevy::render::RenderPlugin {
                    render_creation: low_memory_wgpu().into(),
                    ..default()
                })
                .set(TaskPoolPlugin {
                    task_pool_options: TaskPoolOptions { max_total_threads: 8, ..default() },
                }),
        );
    background::register(&mut app);
    particles::register(&mut app);
    glass::register(&mut app);

    app.insert_resource(Library { games: games.clone(), wishlist })
        .insert_resource(dash)
        .insert_resource(Cfg(cfg))
        .insert_resource(Bg { tx, rx: Mutex::new(rx), art_total: 0, art_done: 0, enhancing: None, busy: false })
        .insert_resource(Opts { screenshot: opts.screenshot, script: opts.script.chars().collect() })
        .insert_resource(Running::default())
        .insert_resource(Sys(sysinfo::System::new()))
        .insert_resource(textures::Textures::new())
        .init_resource::<NavInput>()
        .init_resource::<background::HeroFade>()
        .init_resource::<view::ShelfAnim>()
        .init_resource::<grid::GridAnim>()
        .init_resource::<flight::Flight>()
        .init_resource::<view::InfoAnim>()
        .init_resource::<fx::Fx>()
        .add_systems(PreStartup, load_fonts)
        .add_systems(Startup, (view::spawn_static, setup_sounds, start_scan, flight::spawn))
        .add_systems(
            Update,
            (
                (
                    input::gather_input,
                    autopilot,
                    pump_background,
                    mouse_nav,
                    handle_nav,
                    // Runs while the old layout's scroll is still live, and grounds
                    // cards that landed last frame before the views lay out (no-gap
                    // handoff).
                    flight::start_flight,
                    watch_running,
                    fx::update_fx,
                    view::update_scale,
                    view::sync_tabs,
                    view::sync_shelf,
                    view::animate_tabs,
                    view::animate_shelf,
                )
                    .chain(),
                (
                    (grid::sync_grid, grid::animate_grid, grid::toggle_views).chain(),
                    // After the layouts have run, so the landing slot is where the
                    // card should be this frame.
                    flight::animate_flight,
                    view::update_info,
                    view::update_background,
                    particles::update_particles,
                    (view::update_ambient, view::update_chrome, view::update_launch_splash).chain(),
                    (view::update_details_sheet, glass::update_glass, glass::animate_cards),
                    view::animate_prompts,
                    textures::pump_textures,
                )
                    .chain(),
            )
                .chain(),
        );
    if first_run {
        app.world_mut().resource_mut::<Dash>().status = "Scanning your games…".into();
    }
    app.run();
}

/// DX12 (lighter than Vulkan on Windows) with small GPU allocation blocks. A dashboard
/// draws a few dozen quads, so trading peak allocator speed for memory is free.
/// `WGPU_BACKEND` still overrides the backend.
fn low_memory_wgpu() -> bevy::render::settings::WgpuSettings {
    use bevy::render::settings::{Backends, MemoryHints, WgpuSettings};
    WgpuSettings {
        backends: Some(Backends::from_env().unwrap_or(Backends::DX12)),
        memory_hints: MemoryHints::MemoryUsage,
        ..default()
    }
}

/// Keeps the font assets alive.
#[derive(Resource)]
struct Fonts(#[allow(dead_code)] Vec<Handle<Font>>);

/// Loads just the Windows font files we use instead of indexing every installed font.
/// `SegoeUIVariable.ttf` only exists on Windows 11; on 10 the classic set covers it.
fn load_fonts(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let dir = std::path::Path::new(r"C:\Windows\Fonts");
    let files = [
        "segoeui.ttf",
        "segoeuil.ttf",
        "seguisb.ttf",
        "segoeuib.ttf",
        "SegoeUIVariable.ttf",
        "SegoeIcons.ttf",
        "segmdl2.ttf",
    ];
    let handles = files
        .iter()
        .filter_map(|f| std::fs::read(dir.join(f)).ok())
        .map(|bytes| fonts.add(Font::from_bytes(bytes)))
        .collect();
    commands.insert_resource(Fonts(handles));
}

fn setup_sounds(mut commands: Commands, mut audio: ResMut<Assets<AudioSource>>, dash: Res<Dash>) {
    commands.insert_resource(Sounds::new(&mut audio, dash.sounds));
}

fn start_scan(mut bg: ResMut<Bg>, cfg: Res<Cfg>) {
    spawn_scan(&mut bg, cfg.0.clone(), false);
}

/// Rescans the library and then fills in missing artwork, on a background thread.
fn spawn_scan(bg: &mut Bg, cfg: Config, refresh_art: bool) {
    if bg.busy {
        return;
    }
    bg.busy = true;
    let tx = bg.tx.clone();
    std::thread::spawn(move || {
        let games = library::scan_all(&cfg);
        library::save_cache(&games);
        let wishlist = library::wishlist::load(&cfg, &games);
        let _ = tx.send(BgMsg::Library { games: games.clone(), wishlist: wishlist.clone() });
        // Gamepad support lookups (cached in controllers.json) — the tab fills in as
        // these come back, long before the artwork finishes.
        let games = library::controller::refresh(games);
        library::save_cache(&games);
        let _ = tx.send(BgMsg::Library { games: games.clone(), wishlist: wishlist.clone() });
        if refresh_art {
            art::clear(false);
            library::controller::clear_unknown();
        }
        let (atx, arx) = channel();
        // `fetch_all` pops from the end, so reversed wishlist items go first: the first
        // rows of the Wishlist grid (A onward) get their art before the rest of the list.
        let mut art_games = games;
        art_games.extend(wishlist.into_iter().rev());
        let worker = std::thread::spawn(move || art::fetch_all(art_games, 4, atx));
        for r in arx {
            let _ = tx.send(BgMsg::Art(r));
        }
        let _ = worker.join();
        if cfg.enhance_art {
            run_enhancer(&tx);
        }
        let _ = tx.send(BgMsg::ArtDone);
    });
}

/// Runs `gdash --enhance-art` as a hidden, low-priority child process (so the ML models'
/// memory never lives in the dashboard) and relays its progress.
fn run_enhancer(tx: &Sender<BgMsg>) {
    use std::io::BufRead;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    let Ok(exe) = std::env::current_exe() else { return };
    let child = std::process::Command::new(exe)
        .arg("--enhance-art")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS)
        .spawn();
    let Ok(mut child) = child else { return };
    let Some(out) = child.stdout.take() else { return };
    let (mut done, mut total) = (0, 0);
    for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some("total"), Some(n)) => {
                total = n.parse().unwrap_or(0);
                if total > 0 {
                    let _ = tx.send(BgMsg::Enhancing(0, total));
                }
            }
            (Some("done"), Some(id)) => {
                done += 1;
                let _ = tx.send(BgMsg::Enhanced(id.to_string()));
                let _ = tx.send(BgMsg::Enhancing(done, total));
            }
            (Some("fail"), _) => {
                done += 1;
                let _ = tx.send(BgMsg::Enhancing(done, total));
            }
            _ => {}
        }
    }
    let _ = child.wait();
}

fn pump_background(
    mut bg: ResMut<Bg>,
    mut lib: ResMut<Library>,
    mut dash: ResMut<Dash>,
    mut tex: ResMut<textures::Textures>,
) {
    let msgs: Vec<BgMsg> = bg.rx.lock().unwrap().try_iter().collect();
    for m in msgs {
        match m {
            BgMsg::Library { games, wishlist } => {
                bg.art_total = games.len() + wishlist.len();
                bg.art_done = 0;
                if games != lib.games || wishlist != lib.wishlist {
                    lib.games = games;
                    lib.wishlist = wishlist;
                    dash.rebuild(&lib.games, &lib.wishlist);
                }
                dash.refresh_system_blade(lib.games.len());
            }
            BgMsg::Art(r) => {
                bg.art_done += 1;
                tex.invalidate(&r.id);
            }
            BgMsg::Enhancing(done, total) => bg.enhancing = Some((done, total)),
            BgMsg::Enhanced(id) => tex.reload(&id),
            BgMsg::ArtDone => {
                bg.busy = false;
                bg.enhancing = None;
            }
        }
    }
    dash.status = if !bg.busy {
        String::new()
    } else if let Some((done, total)) = bg.enhancing {
        format!("Enhancing artwork {done}/{total}")
    } else if bg.art_total == 0 {
        "Scanning your games…".into()
    } else {
        format!("Updating artwork {}/{}", bg.art_done, bg.art_total)
    };
}

/// Hover selects tiles; clicks play, switch categories and press on-screen buttons.
#[allow(clippy::too_many_arguments)]
fn mouse_nav(
    mut nav: ResMut<NavInput>,
    mut dash: ResMut<Dash>,
    tiles: Query<(&Tile, &Hovered)>,
    grid_tiles: Query<(&grid::GridTile, &Hovered)>,
    tabs: Query<(&Tab, &Hovered)>,
    caps: Query<(&KeyCap, &Hovered)>,
    play: Query<&Hovered, With<PlayBtn>>,
    details: Query<&Hovered, With<DetailsBtn>>,
    guide: Query<(&GuideRow, &Hovered)>,
    mut commands: Commands,
    sounds: Option<Res<Sounds>>,
) {
    // Hover only steers once the mouse is actually in use (not a bumped desk).
    if !nav.mouse_moved || nav.device != input::Device::Keyboard {
        return;
    }
    let play_sfx = |commands: &mut Commands, s: Sfx| {
        if let Some(snd) = &sounds {
            snd.play(commands, s);
        }
    };
    match dash.mode {
        Mode::Browse => {
            if nav.clicked {
                if let Some((tab, _)) = tabs.iter().find(|(_, h)| h.get()) {
                    if tab.0 != dash.current {
                        dash.current = tab.0;
                        play_sfx(&mut commands, Sfx::Blade);
                    }
                    nav.clicked = false;
                }
                if let Some((cap, _)) = caps.iter().find(|(_, h)| h.get()) {
                    nav.actions.push(if cap.0 < 0 { Nav::PrevBlade } else { Nav::NextBlade });
                    nav.clicked = false;
                }
                if play.iter().any(|h| h.get()) {
                    nav.actions.push(Nav::Accept);
                    nav.clicked = false;
                }
                if details.iter().any(|h| h.get()) {
                    nav.actions.push(Nav::Details);
                    nav.clicked = false;
                }
            }
            let hovered = tiles
                .iter()
                .find(|(_, h)| h.get())
                .map(|(t, _)| t.idx)
                .or_else(|| grid_tiles.iter().find(|(_, h)| h.get()).map(|(t, _)| t.idx));
            if let Some(idx) = hovered {
                let cur = dash.current;
                if idx < dash.blade().entries.len() && dash.sel[cur] != idx {
                    dash.sel[cur] = idx;
                    play_sfx(&mut commands, Sfx::Move);
                }
                if nav.clicked {
                    nav.actions.push(Nav::Accept);
                }
            }
        }
        Mode::Guide => {
            for (row, h) in &guide {
                if h.get() {
                    dash.guide_sel = row.0;
                    if nav.clicked {
                        nav.actions.push(Nav::Accept);
                    }
                }
            }
        }
        Mode::Details => {
            if nav.clicked {
                nav.actions.push(Nav::Back);
            }
        }
        Mode::Running => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_nav(
    mut commands: Commands,
    nav: Res<NavInput>,
    mut dash: ResMut<Dash>,
    mut lib: ResMut<Library>,
    mut bg: ResMut<Bg>,
    mut cfg: ResMut<Cfg>,
    mut running: ResMut<Running>,
    mut tex: ResMut<textures::Textures>,
    mut sounds: Option<ResMut<Sounds>>,
    mut fx: ResMut<fx::Fx>,
    mut exit: MessageWriter<AppExit>,
) {
    let mut sfx: Vec<Sfx> = Vec::new();
    let mut effects = Effects::default();

    // Waking from idle ambient: the first touch racks the interface back in — and is
    // spent doing exactly that. It never presses what it landed on (fx.rs drives the
    // rack; a launch or an overlay can't be open while ambient).
    if fx.ambient > 0.5 && nav.idle == 0.0 {
        return;
    }

    for &a in &nav.actions {
        let mut action = None;
        match dash.mode {
            Mode::Running => {
                if matches!(a, Nav::Back | Nav::Accept) {
                    running.0 = None;
                    dash.mode = Mode::Browse;
                    sfx.push(Sfx::Back);
                }
            }
            Mode::Details => match a {
                Nav::Back | Nav::Details | Nav::Guide => {
                    dash.mode = Mode::Browse;
                    sfx.push(Sfx::Back);
                }
                Nav::Accept => {
                    dash.mode = Mode::Browse;
                    action = dash.selected().map(|e| e.action);
                }
                _ => {}
            },
            Mode::Guide => {
                let n = GUIDE_ITEMS.len() + 1;
                match a {
                    Nav::Up if dash.guide_sel > 0 => {
                        dash.guide_sel -= 1;
                        sfx.push(Sfx::Move);
                    }
                    Nav::Down if dash.guide_sel + 1 < n => {
                        dash.guide_sel += 1;
                        sfx.push(Sfx::Move);
                    }
                    Nav::Back | Nav::Guide => {
                        dash.mode = Mode::Browse;
                        sfx.push(Sfx::Back);
                    }
                    Nav::Accept => {
                        dash.mode = Mode::Browse;
                        if dash.guide_sel == 0 {
                            sfx.push(Sfx::Back);
                        } else {
                            action = Some(GUIDE_ITEMS[dash.guide_sel - 1].1);
                        }
                    }
                    _ => {}
                }
            }
            Mode::Browse => match a {
                Nav::Up | Nav::Down if dash.in_grid() => {
                    if dash.move_row(if a == Nav::Up { -1 } else { 1 }) {
                        sfx.push(Sfx::Move);
                    }
                }
                Nav::Left | Nav::Right | Nav::JumpBack | Nav::JumpForward => {
                    // In the grid a jump is a few rows; on the shelf, five covers.
                    let page = if dash.in_grid() { dash.grid_cols as i32 * 3 } else { 5 };
                    let delta = match a {
                        Nav::Left => -1,
                        Nav::Right => 1,
                        Nav::JumpBack => -page,
                        _ => page,
                    };
                    if dash.move_sel(delta) {
                        sfx.push(Sfx::Move);
                    }
                }
                Nav::PrevBlade if dash.current > 0 => {
                    dash.current -= 1;
                    sfx.push(Sfx::Blade);
                }
                Nav::NextBlade if dash.current + 1 < dash.blades.len() => {
                    dash.current += 1;
                    sfx.push(Sfx::Blade);
                }
                Nav::Back if dash.current != dash.home() => {
                    dash.current = dash.home();
                    sfx.push(Sfx::Blade);
                }
                Nav::Details => {
                    if matches!(
                        dash.selected().map(|e| e.action),
                        Some(Action::Launch(_) | Action::OpenWishlist(_))
                    ) {
                        dash.mode = Mode::Details;
                        sfx.push(Sfx::Open);
                    }
                }
                Nav::Guide => {
                    dash.mode = Mode::Guide;
                    dash.guide_sel = 0;
                    sfx.push(Sfx::Open);
                }
                Nav::Accept => action = dash.selected().map(|e| e.action),
                _ => {}
            },
        }
        if let Some(action) = action {
            let mut ctx = ActionCtx {
                dash: &mut dash,
                lib: &mut lib,
                bg: &mut bg,
                cfg: &mut cfg,
                running: &mut running,
                tex: &mut tex,
            };
            let e = run_action(action, &mut ctx);
            sfx.extend(e.sfx);
            effects.exit |= e.exit;
            if e.launched {
                fx.start_launch();
            }
            if e.pressed {
                fx.start_press();
            }
            if e.sounds_enabled.is_some() {
                effects.sounds_enabled = e.sounds_enabled;
            }
        }
    }

    if let Some(s) = sounds.as_mut() {
        if let Some(on) = effects.sounds_enabled {
            s.enabled = on;
        }
        for x in sfx {
            s.play(&mut commands, x);
        }
    }
    if effects.exit {
        exit.write(AppExit::Success);
    }
}

#[derive(Default)]
struct Effects {
    sfx: Option<Sfx>,
    exit: bool,
    launched: bool,
    /// A game was activated: its cover takes the press dip.
    pressed: bool,
    sounds_enabled: Option<bool>,
}

struct ActionCtx<'a> {
    dash: &'a mut Dash,
    lib: &'a mut Library,
    bg: &'a mut Bg,
    cfg: &'a mut Cfg,
    running: &'a mut Running,
    tex: &'a mut textures::Textures,
}

fn run_action(action: Action, c: &mut ActionCtx) -> Effects {
    let mut fx = Effects { sfx: Some(Sfx::Select), pressed: matches!(action, Action::Launch(_) | Action::OpenWishlist(_)), ..default() };
    match action {
        Action::Launch(i) => {
            let Some(game) = c.lib.games.get(i).cloned() else { return fx };
            match launch::start(&game) {
                Ok(r) => {
                    fx.sfx = Some(Sfx::Launch);
                    fx.launched = true;
                    c.dash.toast = Some((format!("Starting {}…", game.title), 4.0));
                    let now = PlayState::record(&game.id);
                    if let Some(g) = c.lib.games.get_mut(i) {
                        g.last_played = Some(now);
                    }
                    let games = c.lib.games.clone();
                    let wishlist = c.lib.wishlist.clone();
                    c.dash.rebuild(&games, &wishlist);
                    c.dash.mode = Mode::Running;
                    c.running.0 = Some(r);
                }
                Err(e) => {
                    fx.sfx = Some(Sfx::Back);
                    c.dash.toast = Some((format!("Couldn't start {}: {e}", game.title), 6.0));
                }
            }
        }
        Action::OpenWishlist(i) => {
            let Some(game) = c.lib.wishlist.get(i).cloned() else { return fx };
            let Launch::Url { url } = &game.launch else { return fx };
            if crate::win::shell_open(url) {
                let host = library::wishlist::source_host(url);
                c.dash.toast = Some((format!("Opening {host}…"), 3.0));
            } else {
                fx.sfx = Some(Sfx::Back);
                c.dash.toast = Some((format!("Couldn't open {url}"), 6.0));
            }
        }
        Action::Rescan => {
            c.dash.toast = Some(("Rescanning your games…".into(), 2.5));
            spawn_scan(c.bg, c.cfg.0.clone(), false);
        }
        Action::RefreshArt => {
            c.dash.toast = Some(("Looking for missing artwork…".into(), 2.5));
            c.tex.invalidate_all();
            spawn_scan(c.bg, c.cfg.0.clone(), true);
        }
        Action::ToggleAutostart => {
            let on = !c.dash.autostart;
            if crate::win::set_autostart(on) {
                c.dash.autostart = on;
            } else {
                c.dash.toast = Some(("Couldn't change the startup setting".into(), 3.0));
            }
            let n = c.lib.games.len();
            c.dash.refresh_system_blade(n);
        }
        Action::ToggleSounds => {
            c.dash.sounds = !c.dash.sounds;
            c.cfg.0.sounds = c.dash.sounds;
            let _ = c.cfg.0.save();
            fx.sounds_enabled = Some(c.dash.sounds);
            let n = c.lib.games.len();
            c.dash.refresh_system_blade(n);
        }
        Action::OpenConfig => {
            let dir = crate::paths::config_dir();
            crate::win::shell_open(&dir.to_string_lossy());
        }
        Action::Exit => fx.exit = true,
    }
    fx
}

/// Minimises while a game runs and comes back when it exits.
fn watch_running(
    mut running: ResMut<Running>,
    mut dash: ResMut<Dash>,
    mut sys: ResMut<Sys>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut restore_frames: Local<u32>,
) {
    let Ok(mut window) = windows.single_mut() else { return };
    if *restore_frames > 0 {
        *restore_frames -= 1;
        if *restore_frames == 0 {
            crate::win::restore_window(WINDOW_TITLE);
        }
    }
    let Some(r) = running.0.as_mut() else {
        if dash.mode == Mode::Running {
            dash.mode = Mode::Browse;
        }
        return;
    };
    if !r.minimized && r.elapsed() > Duration::from_millis(1500) {
        r.minimized = true;
        window.set_minimized(true);
    }
    if !r.poll(&mut sys.0) {
        running.0 = None;
        dash.mode = Mode::Browse;
        window.set_minimized(false);
        window.focused = true;
        *restore_frames = 3;
    }
}

/// Drives scripted navigation and screenshots (`--nav`, `--screenshot`).
fn autopilot(
    mut commands: Commands,
    mut opts: ResMut<Opts>,
    mut nav: ResMut<NavInput>,
    mut flight: ResMut<flight::Flight>,
    time: Res<Time>,
    bg: Res<Bg>,
    mut fx: ResMut<fx::Fx>,
    mut next: Local<f32>,
    mut shot_at: Local<Option<f32>>,
    mut exit: MessageWriter<AppExit>,
) {
    if opts.screenshot.is_none() && opts.script.is_empty() {
        return;
    }
    let t = time.elapsed_secs();
    // Let the library and textures settle first.
    if t < 3.0 || (bg.busy && t < 20.0) {
        *next = t + 0.5;
        return;
    }
    if t < *next {
        return;
    }
    if !opts.script.is_empty() {
        let c = opts.script.remove(0);
        let a = match c.to_ascii_uppercase() {
            'U' => Some(Nav::Up),
            'D' => Some(Nav::Down),
            'L' => Some(Nav::Left),
            'R' => Some(Nav::Right),
            'Q' => Some(Nav::PrevBlade),
            'E' => Some(Nav::NextBlade),
            'A' => Some(Nav::Accept),
            'B' => Some(Nav::Back),
            'Y' => Some(Nav::Details),
            'G' => Some(Nav::Guide),
            'K' => {
                nav.forced = Some(input::Device::Keyboard);
                None
            }
            'P' => {
                nav.forced = Some(input::Device::Gamepad);
                None
            }
            // The full launch visual without starting anything (splash checks).
            'X' => {
                fx.start_press();
                fx.start_launch();
                None
            }
            // Jump the idle clock: the chrome racks away and the hero takes the screen
            // (ambient checks without waiting out the two minutes).
            'V' => {
                nav.idle = fx::AMBIENT_AFTER + 1.0;
                None
            }
            // Freeze the grid↔shelf cover flight mid-air (morph screenshot checks).
            'F' => {
                flight.freeze = true;
                None
            }
            _ => None,
        };
        nav.actions.extend(a);
        // Scripted input is user input: it wakes ambient like a real touch would.
        if a.is_some() {
            nav.idle = 0.0;
        }
        // Leave time for the last animation to finish before the screenshot.
        // 'W' waits a few seconds (lets the background's particles build up).
        *next = t + match (opts.script.is_empty(), c.eq_ignore_ascii_case(&'X')) {
            _ if c.eq_ignore_ascii_case(&'W') => 4.0,
            (true, true) => 0.25,
            (true, false) => 1.2,
            _ => 0.3,
        };
        return;
    }
    match (*shot_at, opts.screenshot.clone()) {
        (None, Some(path)) => {
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            *shot_at = Some(t);
        }
        (Some(s), _) if t > s + 1.5 => {
            exit.write(AppExit::Success);
        }
        (None, None) => {}
        _ => {}
    }
}

