//! Dashboard state: blades, entries, selection.

use bevy::prelude::*;

use super::theme;
use crate::library::{ControllerSupport, Game, Launch, System};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Launch(usize),
    /// A wishlist item: open its source page in the browser.
    OpenWishlist(usize),
    Rescan,
    RefreshArt,
    ToggleAutostart,
    ToggleSounds,
    OpenConfig,
    Exit,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub title: String,
    pub subtitle: String,
    pub action: Action,
    /// Longer text for the detail pane (system actions).
    pub detail: String,
    /// Icon-font glyph for non-game entries.
    pub glyph: Option<char>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BladeKey {
    /// Every game at once, as a grid.
    All,
    Recent,
    System(System),
    /// Every game again, but sorted and labelled by gamepad support.
    Controller,
    /// Games from the to-download list that aren't installed.
    Wishlist,
    Settings,
}

#[derive(Clone, Debug)]
pub struct Blade {
    pub key: BladeKey,
    pub name: String,
    pub color: Color,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Browse,
    Guide,
    Details,
    Running,
}

#[derive(Resource)]
pub struct Library {
    pub games: Vec<Game>,
    /// Not installed; comes from the user's to-download file.
    pub wishlist: Vec<Game>,
}

#[derive(Resource)]
pub struct Dash {
    pub blades: Vec<Blade>,
    pub current: usize,
    pub sel: Vec<usize>,
    pub mode: Mode,
    pub guide_sel: usize,
    /// Bumped whenever the blade list itself changes (tabs need respawning).
    pub layout_gen: u64,
    /// Bumped whenever any blade's entries change (shelf tiles need respawning).
    pub content_gen: u64,
    pub status: String,
    pub autostart: bool,
    pub sounds: bool,
    pub toast: Option<(String, f32)>,
    /// Columns in the All grid (set by the grid layout, used for up/down).
    pub grid_cols: usize,
}

pub const GUIDE_ITEMS: [(&str, Action); 4] = [
    ("Rescan library", Action::Rescan),
    ("Refresh artwork", Action::RefreshArt),
    ("Open config folder", Action::OpenConfig),
    ("Exit gdash", Action::Exit),
];

impl Dash {
    pub fn new(games: &[Game], wishlist: &[Game], autostart: bool, sounds: bool) -> Self {
        let mut d = Dash {
            blades: Vec::new(),
            current: 0,
            sel: Vec::new(),
            mode: Mode::Browse,
            guide_sel: 0,
            layout_gen: 0,
            content_gen: 0,
            status: String::new(),
            autostart,
            sounds,
            toast: None,
            grid_cols: 1,
        };
        d.rebuild(games, wishlist);
        d
    }

    pub fn blade(&self) -> &Blade {
        &self.blades[self.current]
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.blade().entries.get(self.sel[self.current])
    }

    pub fn in_grid(&self) -> bool {
        matches!(
            self.blade().key,
            BladeKey::All | BladeKey::Wishlist | BladeKey::Controller
        )
    }

    /// The blade the dashboard starts on and Back returns to: Recent, else the first system.
    pub fn home(&self) -> usize {
        self.blades
            .iter()
            .position(|b| b.key == BladeKey::Recent)
            .or_else(|| self.blades.iter().position(|b| matches!(b.key, BladeKey::System(_))))
            .unwrap_or(0)
    }

    /// Moves the grid selection one row up (-1) or down (1). Moving down into a short last
    /// row lands on its last game. Returns whether it moved.
    pub fn move_row(&mut self, dir: i32) -> bool {
        let n = self.blade().entries.len();
        let cols = self.grid_cols.max(1);
        let cur = self.sel[self.current];
        let next = if dir < 0 {
            if cur < cols { return false; }
            cur - cols
        } else {
            if cur / cols == (n.saturating_sub(1)) / cols { return false; }
            (cur + cols).min(n - 1)
        };
        self.sel[self.current] = next;
        true
    }

    /// Rebuilds blades from the library, keeping the current blade and selection when possible.
    pub fn rebuild(&mut self, games: &[Game], wishlist: &[Game]) {
        let prev_key = self.blades.get(self.current).map(|b| b.key.clone());
        let prev_sel: Vec<(BladeKey, String)> = self
            .blades
            .iter()
            .zip(&self.sel)
            .filter_map(|(b, &s)| b.entries.get(s).map(|e| (b.key.clone(), e.title.clone())))
            .collect();

        let mut blades = Vec::new();
        if !games.is_empty() {
            let mut all: Vec<usize> = (0..games.len()).collect();
            all.sort_by_key(|&i| games[i].sort_key());
            blades.push(Blade {
                key: BladeKey::All,
                name: "All".into(),
                color: theme::ALL,
                entries: all.iter().map(|&i| game_entry(games, i, true)).collect(),
            });
        }
        let mut recent: Vec<usize> = (0..games.len()).filter(|&i| games[i].last_played.is_some()).collect();
        recent.sort_by_key(|&i| std::cmp::Reverse(games[i].last_played));
        recent.truncate(15);
        if !recent.is_empty() {
            blades.push(Blade {
                key: BladeKey::Recent,
                name: "Recent".into(),
                color: theme::RECENT,
                entries: recent.iter().map(|&i| game_entry(games, i, true)).collect(),
            });
        }
        for sys in System::ALL {
            let mut idx: Vec<usize> = (0..games.len()).filter(|&i| games[i].system == sys).collect();
            if idx.is_empty() {
                continue;
            }
            idx.sort_by_key(|&i| games[i].sort_key());
            blades.push(Blade {
                key: BladeKey::System(sys),
                name: sys.label().into(),
                color: match sys {
                    System::Steam => theme::STEAM,
                    System::Pc => theme::PC,
                    System::Xbox => theme::XBOX,
                    System::Xbox360 => theme::XBOX360,
                    System::Wishlist => theme::WISHLIST,
                },
                entries: idx.iter().map(|&i| game_entry(games, i, false)).collect(),
            });
        }
        if !games.is_empty() {
            // Only games that actually work with a gamepad: full support first, then
            // partial, alphabetical within each. Unknown and "no support" games stay out.
            let playable = |g: &Game| matches!(g.controller, Some(ControllerSupport::Full | ControllerSupport::Partial));
            let mut idx: Vec<usize> = (0..games.len()).filter(|&i| playable(&games[i])).collect();
            if !idx.is_empty() {
                idx.sort_by_key(|&i| (std::cmp::Reverse(games[i].controller), games[i].sort_key()));
                blades.push(Blade {
                    key: BladeKey::Controller,
                    name: "Controller".into(),
                    color: theme::CONTROLLER,
                    entries: idx.iter().map(|&i| controller_entry(games, i)).collect(),
                });
            }
        }
        if !wishlist.is_empty() {
            let mut idx: Vec<usize> = (0..wishlist.len()).collect();
            idx.sort_by_key(|&i| wishlist[i].sort_key());
            blades.push(Blade {
                key: BladeKey::Wishlist,
                name: "Wishlist".into(),
                color: theme::WISHLIST,
                entries: idx.iter().map(|&i| wish_entry(wishlist, i)).collect(),
            });
        }
        blades.push(Blade {
            key: BladeKey::Settings,
            name: "System".into(),
            color: theme::SYSTEM,
            entries: self.system_entries(games.len()),
        });

        let changed = blades.len() != self.blades.len()
            || blades.iter().zip(&self.blades).any(|(a, b)| a.key != b.key);
        self.sel = blades
            .iter()
            .map(|b| {
                prev_sel
                    .iter()
                    .find(|(k, _)| *k == b.key)
                    .and_then(|(_, t)| b.entries.iter().position(|e| &e.title == t))
                    .unwrap_or(0)
            })
            .collect();
        let prev = prev_key.and_then(|k| blades.iter().position(|b| b.key == k));
        self.blades = blades;
        self.current = prev.unwrap_or_else(|| self.home());
        if changed {
            self.layout_gen += 1;
        }
        self.content_gen += 1;
    }

    pub fn refresh_system_blade(&mut self, game_count: usize) {
        let entries = self.system_entries(game_count);
        if let Some(b) = self.blades.iter_mut().find(|b| b.key == BladeKey::Settings) {
            let changed = b.entries.iter().map(|e| &e.title).ne(entries.iter().map(|e| &e.title))
                || b.entries.iter().map(|e| &e.subtitle).ne(entries.iter().map(|e| &e.subtitle));
            b.entries = entries;
            if changed {
                self.content_gen += 1;
            }
        }
    }

    fn system_entries(&self, game_count: usize) -> Vec<Entry> {
        let on_off = |b: bool| if b { "On" } else { "Off" };
        vec![
            Entry {
                glyph: Some('\u{E72C}'),
                title: "Rescan library".into(),
                subtitle: format!("{game_count} games found"),
                action: Action::Rescan,
                detail: "Look for newly installed Steam games, PC game folders and emulator images.".into(),
            },
            Entry {
                glyph: Some('\u{EB9F}'),
                title: "Refresh artwork".into(),
                subtitle: "Retry games with missing art".into(),
                action: Action::RefreshArt,
                detail: "Search again for box art, backgrounds, logos and icons that weren't found before, \
                         and retry controller-support lookups that came back unknown. \
                         To use your own image, put cover.jpg / hero.jpg / logo.png / icon.png in the \
                         game's art folder."
                    .into(),
            },
            Entry {
                glyph: Some('\u{E7E8}'),
                title: format!("Start with Windows: {}", on_off(self.autostart)),
                subtitle: "Boot straight into the dashboard".into(),
                action: Action::ToggleAutostart,
                detail: "Launch gdash automatically when you sign in to Windows.".into(),
            },
            Entry {
                glyph: Some('\u{E767}'),
                title: format!("Interface sounds: {}", on_off(self.sounds)),
                subtitle: "Navigation clicks and chimes".into(),
                action: Action::ToggleSounds,
                detail: "Play short sounds when moving, switching blades and launching games.".into(),
            },
            Entry {
                glyph: Some('\u{E8B7}'),
                title: "Open config folder".into(),
                subtitle: "config.toml: paths, overrides, hidden games".into(),
                action: Action::OpenConfig,
                detail: format!(
                    "Edit scan folders, emulator paths, per-game exe/title overrides and hidden games.\n\n{}",
                    crate::paths::config_file().display()
                ),
            },
            Entry {
                glyph: Some('\u{E89F}'),
                title: "Exit gdash".into(),
                subtitle: "Back to Windows".into(),
                action: Action::Exit,
                detail: "Close the dashboard.".into(),
            },
        ]
    }

    /// Moves the selection along the shelf. Returns whether it moved.
    pub fn move_sel(&mut self, delta: i32) -> bool {
        let n = self.blade().entries.len();
        if n == 0 {
            return false;
        }
        let cur = self.sel[self.current] as i32;
        let next = (cur + delta).clamp(0, n as i32 - 1) as usize;
        if next == cur as usize {
            return false;
        }
        self.sel[self.current] = next;
        true
    }
}

/// The Controller blade: same game, but the meta line is its gamepad support.
fn controller_entry(games: &[Game], i: usize) -> Entry {
    let g = &games[i];
    Entry {
        title: g.title.clone(),
        subtitle: g.controller_label().to_string(),
        action: Action::Launch(i),
        detail: String::new(),
        glyph: None,
    }
}

fn wish_entry(wishlist: &[Game], i: usize) -> Entry {
    let g = &wishlist[i];
    let (subtitle, detail) = match &g.launch {
        Launch::Url { url } => (
            format!("Not installed  •  {}", crate::library::wishlist::source_host(url)),
            url.clone(),
        ),
        _ => ("Not installed".into(), String::new()),
    };
    Entry {
        title: g.title.clone(),
        subtitle,
        action: Action::OpenWishlist(i),
        detail,
        glyph: None,
    }
}

fn game_entry(games: &[Game], i: usize, show_system: bool) -> Entry {
    let g = &games[i];
    let mut sub = Vec::new();
    if show_system {
        sub.push(g.system.label().to_string());
    }
    match g.last_played {
        Some(t) => sub.push(format!("Played {}", ago(t))),
        None => sub.push(if show_system { String::new() } else { g.system.label().to_string() }),
    }
    sub.retain(|s| !s.is_empty());
    Entry {
        title: g.title.clone(),
        subtitle: sub.join("  •  "),
        action: Action::Launch(i),
        detail: String::new(),
        glyph: None,
    }
}

pub fn ago(t: u64) -> String {
    let now = crate::library::now_secs();
    let d = now.saturating_sub(t);
    match d {
        0..=119 => "just now".into(),
        120..=3599 => format!("{} minutes ago", d / 60),
        3600..=7199 => "an hour ago".into(),
        7200..=86_399 => format!("{} hours ago", d / 3600),
        86_400..=172_799 => "yesterday".into(),
        172_800..=2_591_999 => format!("{} days ago", d / 86_400),
        2_592_000..=5_183_999 => "last month".into(),
        2_592_000.. if d < 31_536_000 => format!("{} months ago", d / 2_592_000),
        _ => format!("{} years ago", (d / 31_536_000).max(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn game(id: &str, title: &str, system: System, controller: Option<ControllerSupport>) -> Game {
        Game {
            id: id.into(),
            title: title.into(),
            system,
            launch: Launch::Steam { appid: 1, install_dir: PathBuf::new() },
            steam_appid: Some(1),
            art_name: None,
            last_played: None,
            controller,
        }
    }

    #[test]
    fn controller_blade_shows_only_games_that_work() {
        let games = vec![
            game("a", "Beta", System::Steam, Some(ControllerSupport::None)),
            game("b", "Alpha", System::Pc, Some(ControllerSupport::Full)),
            game("c", "Gamma", System::Steam, None),
            game("d", "Delta", System::Xbox, Some(ControllerSupport::Full)),
            game("e", "Zeta", System::Pc, Some(ControllerSupport::Partial)),
        ];
        let mut d = Dash::new(&games, &[], false, false);
        {
            let blade = d.blades.iter().find(|b| b.key == BladeKey::Controller).expect("controller blade");
            // Full support first (alphabetical), then partial. No-support and unknown
            // games are left out entirely.
            let titles: Vec<&str> = blade.entries.iter().map(|e| e.title.as_str()).collect();
            assert_eq!(titles, ["Alpha", "Delta", "Zeta"]);
            assert_eq!(blade.entries[0].subtitle, "Full controller support");
            assert_eq!(blade.entries[2].subtitle, "Partial controller support");
            assert!(matches!(blade.entries[0].action, Action::Launch(1))); // Alpha = games[1]
            assert!(matches!(blade.entries[2].action, Action::Launch(4))); // Zeta = games[4]
        }
        // It sits after the system blades, before the System blade, and shows as a grid.
        let keys: Vec<BladeKey> = d.blades.iter().map(|b| b.key.clone()).collect();
        let at = keys.iter().position(|k| *k == BladeKey::Controller).unwrap();
        assert!(matches!(keys[at - 1], BladeKey::System(_)));
        assert_eq!(keys[at + 1], BladeKey::Settings);
        d.current = at;
        assert!(d.in_grid());
    }

    #[test]
    fn controller_blade_hides_itself_when_nothing_is_playable() {
        let games = vec![game("a", "Beta", System::Steam, Some(ControllerSupport::None))];
        let d = Dash::new(&games, &[], false, false);
        assert!(d.blades.iter().all(|b| b.key != BladeKey::Controller));
    }
}
