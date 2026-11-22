//! Which games you can actually play with a controller.
//!
//! Best effort, most reliable first:
//!
//! 1. `controller = "full" | "partial" | "none"` under `[overrides.<id>]` in config.toml
//!    (applied in `scan_all`, always wins).
//! 2. The Steam store page of a known app id — its `controller_support` tag.
//! 3. For PC games without one: a Steam store search by title, then that page's tag.
//! 4. The game's own files: XInput / DirectInput / SDL game-controller DLLs and configs
//!    next to its exe mean a gamepad works at least in game (Partial).
//!
//! Emulated Xbox games are console games and always count as Full (set in `scan_all`).
//! Lookups are cached in `%LOCALAPPDATA%\gdash\data\controllers.json`; `null` marks a
//! game that was checked but nothing was found, so it isn't retried every launch.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::art::Fetcher;
use crate::library::{ControllerSupport, Game, Launch, System};
use crate::paths;

/// Game ids we've looked at → support (or `None` = checked, nothing found).
type Cache = HashMap<String, Option<ControllerSupport>>;

fn cache_file() -> PathBuf {
    paths::data_dir().join("controllers.json")
}

fn load_cache() -> Cache {
    std::fs::read_to_string(cache_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_cache(cache: &Cache) {
    let file = cache_file();
    let _ = std::fs::create_dir_all(file.parent().unwrap());
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(file, json);
    }
}

/// Fills in unknown `controller` values from the cache. Values already set (config
/// overrides, console games) are left alone.
pub fn merge_cached(games: &mut [Game]) {
    let cache = load_cache();
    merge_from(games, &cache);
}

fn merge_from(games: &mut [Game], cache: &Cache) {
    for g in games.iter_mut() {
        if g.controller.is_none() {
            g.controller = cache.get(&g.id).copied().flatten();
        }
    }
}

/// Drops "checked but unknown" entries so the next scan looks those games up again.
/// Used by Refresh artwork, the retry-everything button.
pub fn clear_unknown() {
    let mut cache = load_cache();
    cache.retain(|_, v| v.is_some());
    save_cache(&cache);
}

/// Looks up controller support for games that don't have it yet, on a few worker
/// threads. Returns the same games with `controller` filled in where possible.
pub fn refresh(games: Vec<Game>) -> Vec<Game> {
    let games = Arc::new(games);
    let cache = load_cache();
    // Only Steam and PC games need a lookup: consoles are set in `scan_all`, and
    // a value already present is an override or a past lookup.
    let todo: Vec<usize> = games
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            g.controller.is_none()
                && !cache.contains_key(&g.id)
                && matches!(g.system, System::Steam | System::Pc)
        })
        .map(|(i, _)| i)
        .collect();
    if todo.is_empty() {
        return Arc::try_unwrap(games).unwrap_or_default();
    }

    let queue = Arc::new(Mutex::new(todo.into_iter().rev().collect::<Vec<usize>>()));
    let found: Arc<Mutex<Cache>> = Arc::new(Mutex::new(Cache::new()));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let queue = queue.clone();
        let found = found.clone();
        let games = games.clone();
        handles.push(std::thread::spawn(move || {
            let fetcher = Fetcher::new();
            loop {
                let Some(i) = queue.lock().unwrap().pop() else { break };
                let g = &games[i];
                let level = detect(&fetcher, g);
                found.lock().unwrap().insert(g.id.clone(), level);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }

    let found = Arc::try_unwrap(found).ok().and_then(|m| m.into_inner().ok()).unwrap_or_default();
    let mut games = Arc::try_unwrap(games).ok().unwrap_or_default();
    let mut all: Cache = load_cache();
    merge_from(&mut games, &found);
    all.extend(found);
    save_cache(&all);
    games
}

/// The lookup for one game: the Steam store's controller-support info, backed up by
/// the game's own files. `None` = couldn't tell.
fn detect(fetcher: &Fetcher, game: &Game) -> Option<ControllerSupport> {
    match game.system {
        System::Xbox | System::Xbox360 => Some(ControllerSupport::Full),
        System::Steam => game
            .steam_appid
            .and_then(|id| fetcher.app_details(id))
            .map(|d| steam_level(d.controller_tag.as_deref(), &d.categories)),
        System::Pc => {
            // Direct evidence from the game's own folder, always available offline.
            let local = scan_local(game);
            let store = store_level(fetcher, game);
            match (store, local) {
                // A tagged store page wins, but local files can upgrade "no support"
                // to Partial: the page may be the wrong game, or just untagged.
                (Some(s), l) => Some(match l {
                    Some(l) => s.max(l),
                    None => s,
                }),
                (None, l) => l,
            }
        }
        System::Wishlist => None,
    }
}

/// A PC game's store-page level: a `steam_appid.txt` id when the store name matches
/// (they're often DLC or test ids), else a store search by title.
fn store_level(fetcher: &Fetcher, game: &Game) -> Option<ControllerSupport> {
    if let Some(appid) = game.steam_appid {
        if let Some(d) = fetcher.app_details(appid) {
            if crate::art::titles_match(&game.title, &d.name) {
                return Some(steam_level(d.controller_tag.as_deref(), &d.categories));
            }
        }
    }
    fetcher
        .store_search(&game.title, false)
        .and_then(|appid| fetcher.app_details(appid))
        .map(|d| steam_level(d.controller_tag.as_deref(), &d.categories))
}

/// A store page's verdict. The `controller_support` tag is the short form; the
/// category badges (28 = full, 18 = partial) are what the page actually shows and
/// cover the games whose tag field is missing. No either = no support listed.
fn steam_level(tag: Option<&str>, categories: &[u32]) -> ControllerSupport {
    match tag.map(str::to_ascii_lowercase).as_deref() {
        Some("full") => return ControllerSupport::Full,
        Some("partial") => return ControllerSupport::Partial,
        _ => {}
    }
    if categories.contains(&28) {
        ControllerSupport::Full
    } else if categories.contains(&18) {
        ControllerSupport::Partial
    } else {
        ControllerSupport::None
    }
}

/// Looks for controller DLLs and mapping files in a PC game's folder: XInput is what
/// Xbox-style pads speak, DirectInput is the older API, and a game-controller DB means
/// the SDL2 controller API.
fn scan_local(game: &Game) -> Option<ControllerSupport> {
    let Launch::Exe { exe, cwd, .. } = &game.launch else { return None };
    let mut dirs: Vec<PathBuf> = vec![cwd.clone()];
    if let Some(p) = exe.parent().filter(|p| !dirs.as_slice().contains(&p.to_path_buf())) {
        dirs.push(p.to_path_buf());
    }
    // Games often keep them one level down, in a binaries folder.
    const SUBDIR_HINTS: [&str; 6] = ["bin", "win", "x64", "x86", "data", "engine"];
    if let Ok(rd) = std::fs::read_dir(cwd) {
        for e in rd.flatten().take(512) {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if e.path().is_dir() && SUBDIR_HINTS.iter().any(|h| name.contains(h)) {
                dirs.push(e.path());
                if dirs.len() >= 12 {
                    break;
                }
            }
        }
    }
    for dir in &dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.starts_with("xinput") && name.ends_with(".dll") {
                return Some(ControllerSupport::Partial);
            }
            if name == "dinput8.dll" || name.contains("gamecontrollerdb") {
                return Some(ControllerSupport::Partial);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::titles::clean_title;
    use std::path::Path;

    fn pc_game(dir: &Path) -> Game {
        Game {
            id: "pc-test".into(),
            title: clean_title("Test Game"),
            system: System::Pc,
            launch: Launch::Exe {
                exe: dir.join("Game.exe"),
                args: Vec::new(),
                cwd: dir.to_path_buf(),
                watch_dir: Some(dir.to_path_buf()),
            },
            steam_appid: None,
            art_name: None,
            last_played: None,
            controller: None,
            accent: None,
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gdash-test-{}", tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_override_values() {
        assert_eq!(ControllerSupport::parse("full"), Some(ControllerSupport::Full));
        assert_eq!(ControllerSupport::parse(" Partial "), Some(ControllerSupport::Partial));
        assert_eq!(ControllerSupport::parse("none"), Some(ControllerSupport::None));
        assert_eq!(ControllerSupport::parse("maybe"), None);
    }

    #[test]
    fn full_beats_partial_beats_none() {
        assert!(ControllerSupport::Full > ControllerSupport::Partial);
        assert!(ControllerSupport::Partial > ControllerSupport::None);
        // A store "no support" page still upgrades to Partial when the game's own
        // files talk to a controller.
        assert_eq!(ControllerSupport::None.max(ControllerSupport::Partial), ControllerSupport::Partial);
    }

    #[test]
    fn store_tag_maps_to_levels() {
        assert_eq!(steam_level(Some("full"), &[]), ControllerSupport::Full);
        assert_eq!(steam_level(Some("Partial"), &[]), ControllerSupport::Partial);
        assert_eq!(steam_level(None, &[]), ControllerSupport::None);
        assert_eq!(steam_level(Some("weird"), &[]), ControllerSupport::None);
    }

    #[test]
    fn category_badges_back_up_the_missing_tag() {
        // HELLDIVERS 2 has no controller_support tag, only category 18.
        assert_eq!(steam_level(None, &[18, 62]), ControllerSupport::Partial);
        assert_eq!(steam_level(None, &[28]), ControllerSupport::Full);
        // The tag wins when both exist.
        assert_eq!(steam_level(Some("full"), &[18]), ControllerSupport::Full);
    }

    #[test]
    fn xinput_dlls_mean_partial_support() {
        let dir = scratch("xinput");
        std::fs::write(dir.join("xinput1_3.dll"), b"dll").unwrap();
        assert_eq!(scan_local(&pc_game(&dir)), Some(ControllerSupport::Partial));
    }

    #[test]
    fn sdl_game_controller_db_in_a_bin_folder_counts() {
        let dir = scratch("sdl");
        let bin = dir.join("Binaries");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("gamecontrollerdb.txt"), b"map").unwrap();
        assert_eq!(scan_local(&pc_game(&dir)), Some(ControllerSupport::Partial));
    }

    #[test]
    fn plain_folders_and_steam_games_find_nothing() {
        let dir = scratch("plain");
        assert_eq!(scan_local(&pc_game(&dir)), None);
        let steam = Game {
            id: "steam-1".into(),
            title: "S".into(),
            system: System::Steam,
            launch: Launch::Steam { appid: 1, install_dir: PathBuf::new() },
            steam_appid: Some(1),
            art_name: None,
            last_played: None,
            controller: None,
            accent: None,
        };
        assert_eq!(scan_local(&steam), None);
    }

    #[test]
    fn cache_merges_without_overriding_set_values() {
        let mut games = vec![
            Game { id: "a".into(), controller: Some(ControllerSupport::Full), ..pc_game(Path::new("a")) },
            Game { id: "b".into(), controller: None, ..pc_game(Path::new("b")) },
            Game { id: "c".into(), controller: None, ..pc_game(Path::new("c")) },
        ];
        let cache: Cache = [
            ("b".into(), Some(ControllerSupport::Partial)),
            ("c".into(), None), // checked, unknown
        ]
        .into_iter()
        .collect();
        merge_from(&mut games, &cache);
        assert_eq!(games[0].controller, Some(ControllerSupport::Full)); // untouched
        assert_eq!(games[1].controller, Some(ControllerSupport::Partial));
        assert_eq!(games[2].controller, None); // stays unknown: cache said so
    }
}
