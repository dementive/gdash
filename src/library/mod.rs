//! Discovers installed games across Steam, native PC folders and emulators.

pub mod controller;
pub mod emu;
pub mod native;
pub mod steam;
pub mod titles;
pub mod vdf;
pub mod wishlist;

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::paths;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum System {
    Steam,
    Pc,
    Xbox,
    Xbox360,
    /// Not installed; listed from the user's to-download file.
    Wishlist,
}

impl System {
    pub const ALL: [System; 4] = [System::Steam, System::Pc, System::Xbox, System::Xbox360];

    pub fn label(self) -> &'static str {
        match self {
            System::Steam => "Steam",
            System::Pc => "PC",
            System::Xbox => "Xbox",
            System::Xbox360 => "Xbox 360",
            System::Wishlist => "Wishlist",
        }
    }
}

/// What gdash could find out about playing a game with a controller. `None` on `Game`
/// means unknown: not checked yet, or nothing conclusive found. Variant order is also
/// the quality order: `Full` is the best verdict, so `max()` merges evidence upward.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ControllerSupport {
    /// Listed as having no controller support.
    None,
    /// A gamepad works in game, but some parts may need a keyboard and mouse.
    Partial,
    /// Plays start to finish on a gamepad, menus included.
    Full,
}

impl ControllerSupport {
    /// The subtitle shown on the Controller tab. `None` (unknown) reads "Controller support unknown".
    pub fn label(self) -> &'static str {
        match self {
            ControllerSupport::Full => "Full controller support",
            ControllerSupport::Partial => "Partial controller support",
            ControllerSupport::None => "No controller support",
        }
    }

    /// Parses a config override value, case-insensitively.
    pub fn parse(s: &str) -> Option<ControllerSupport> {
        match s.trim().to_ascii_lowercase().as_str() {
            "full" | "complete" | "yes" | "true" => Some(ControllerSupport::Full),
            "partial" | "some" => Some(ControllerSupport::Partial),
            "none" | "no" | "false" | "keyboard" => Some(ControllerSupport::None),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Launch {
    /// `steam://rungameid/<appid>`
    Steam { appid: u32, install_dir: PathBuf },
    /// Run an executable directly.
    Exe {
        exe: PathBuf,
        args: Vec<String>,
        cwd: PathBuf,
        /// Folder whose processes count as "the game is running" (for launcher stubs).
        watch_dir: Option<PathBuf>,
    },
    /// A wishlist entry: opens this page in the browser instead of starting a game.
    Url { url: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Game {
    pub id: String,
    pub title: String,
    pub system: System,
    pub launch: Launch,
    /// Steam app id, used for artwork (and for launching Steam games).
    pub steam_appid: Option<u32>,
    /// Original No-Intro style name, used for libretro box art lookups.
    pub art_name: Option<String>,
    /// Unix seconds.
    pub last_played: Option<u64>,
    /// Gamepad support, when gdash could find it. `None` = unknown.
    #[serde(default)]
    pub controller: Option<ControllerSupport>,
    /// Signature colour pinned by hand in config.toml (`accent = "#rrggbb"` under the
    /// game's `[overrides.<id>]`), used instead of the one extracted from its art.
    #[serde(default)]
    pub accent: Option<String>,
}

impl Game {
    pub fn sort_key(&self) -> String {
        let t = self.title.to_lowercase();
        t.strip_prefix("the ").map(str::to_string).unwrap_or(t)
    }

    /// The Controller tab's label for this game, including "unknown".
    pub fn controller_label(&self) -> &'static str {
        self.controller.map(ControllerSupport::label).unwrap_or("Controller support unknown")
    }
}

/// Runs every scanner, applies overrides/hidden and merges play history.
pub fn scan_all(cfg: &Config) -> Vec<Game> {
    let mut games = Vec::new();
    if cfg.steam {
        games.extend(steam::scan(cfg.steam_path.as_deref()));
    }
    for root in &cfg.pc_roots {
        games.extend(native::scan_root(root, cfg));
    }
    if let Some(xemu) = &cfg.xemu {
        games.extend(emu::scan_xbox(xemu));
    }
    if let Some(xenia) = &cfg.xenia {
        games.extend(emu::scan_x360(xenia));
    }

    for g in &mut games {
        if let Some(ov) = cfg.overrides.get(&g.id) {
            if let Some(t) = &ov.title {
                g.title = t.clone();
            }
            if let Some(id) = ov.steam_appid {
                g.steam_appid = Some(id);
            }
            if let Some(c) = ov.controller.as_deref().and_then(ControllerSupport::parse) {
                g.controller = Some(c);
            }
            if let Some(a) = &ov.accent {
                g.accent = Some(a.clone());
            }
        }
    }
    games.retain(|g| !cfg.hidden.contains(&g.id));

    // Unique ids (two folders could slug the same).
    let mut seen: HashMap<String, usize> = HashMap::new();
    for g in &mut games {
        let n = seen.entry(g.id.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            g.id = format!("{}-{}", g.id, n);
        }
    }

    // Controller support: cached lookups first (config overrides above already win),
    // and console games are made for gamepads.
    controller::merge_cached(&mut games);
    for g in &mut games {
        if g.controller.is_none() && matches!(g.system, System::Xbox | System::Xbox360) {
            g.controller = Some(ControllerSupport::Full);
        }
    }

    let state = PlayState::load();
    for g in &mut games {
        if let Some(&t) = state.last_played.get(&g.id) {
            g.last_played = Some(g.last_played.map_or(t, |s| s.max(t)));
        }
    }
    games.sort_by_key(|g| (g.system, g.sort_key()));
    games
}

pub fn load_cache() -> Option<Vec<Game>> {
    let text = std::fs::read_to_string(paths::library_cache_file()).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save_cache(games: &[Game]) {
    let file = paths::library_cache_file();
    let _ = std::fs::create_dir_all(file.parent().unwrap());
    if let Ok(json) = serde_json::to_string(games) {
        let _ = std::fs::write(file, json);
    }
}

/// Launch history that gdash records itself (Steam keeps its own too).
#[derive(Serialize, Deserialize, Default)]
pub struct PlayState {
    pub last_played: HashMap<String, u64>,
}

impl PlayState {
    pub fn load() -> PlayState {
        std::fs::read_to_string(paths::state_file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn record(id: &str) -> u64 {
        let now = now_secs();
        let mut st = PlayState::load();
        st.last_played.insert(id.to_string(), now);
        let file = paths::state_file();
        let _ = std::fs::create_dir_all(file.parent().unwrap());
        if let Ok(json) = serde_json::to_string_pretty(&st) {
            let _ = std::fs::write(file, json);
        }
        now
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
