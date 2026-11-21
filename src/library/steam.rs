//! Installed Steam games from `libraryfolders.vdf` and `appmanifest_*.acf`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::vdf::{self, Vdf};
use super::{Game, Launch, System};

/// App ids that are tools/runtimes rather than games.
const TOOL_APPIDS: &[u32] = &[228980, 1070560, 1391110, 1628350, 1493710, 2180100, 1826330];

pub fn steam_root(override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = override_path {
        return Some(p.to_path_buf());
    }
    #[cfg(windows)]
    {
        if let Some(p) = crate::win::reg_string(r"Software\Valve\Steam", "SteamPath") {
            let p = PathBuf::from(p.replace('/', "\\"));
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    let default = PathBuf::from(r"C:\Program Files (x86)\Steam");
    default.is_dir().then_some(default)
}

pub fn scan(override_path: Option<&Path>) -> Vec<Game> {
    let Some(root) = steam_root(override_path) else {
        return Vec::new();
    };
    let last_played = read_last_played(&root);
    let mut games = Vec::new();
    for lib in library_folders(&root) {
        let apps = lib.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&apps) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            let doc = vdf::parse(&text);
            let Some(app) = doc.get("AppState") else { continue };
            let Some(appid) = app.str("appid").and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let title = app.str("name").unwrap_or("").to_string();
            let installdir = app.str("installdir").unwrap_or("").to_string();
            if is_tool(appid, &title, &installdir) {
                continue;
            }
            games.push(Game {
                id: format!("steam-{appid}"),
                title: title.replace('™', "").replace('®', "").trim().to_string(),
                system: System::Steam,
                launch: Launch::Steam { appid, install_dir: apps.join("common").join(&installdir) },
                steam_appid: Some(appid),
                art_name: None,
                last_played: last_played.get(&appid).copied().filter(|&t| t > 0),
                controller: None,
            });
        }
    }
    games
}

fn is_tool(appid: u32, name: &str, installdir: &str) -> bool {
    let n = name.to_lowercase();
    TOOL_APPIDS.contains(&appid)
        || installdir.eq_ignore_ascii_case("Steamworks Shared")
        || n.contains("redistributable")
        || n.starts_with("proton")
        || n.starts_with("steam linux runtime")
        || n.contains("dedicated server")
        || n.ends_with(" sdk")
}

fn library_folders(root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![root.to_path_buf()];
    let file = root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(file) {
        let doc = vdf::parse(&text);
        if let Some(Vdf::Obj(items)) = doc.get("libraryfolders") {
            for (_, lib) in items {
                if let Some(p) = lib.str("path") {
                    let p = PathBuf::from(p);
                    if !libs.iter().any(|l| same_path(l, &p)) {
                        libs.push(p);
                    }
                }
            }
        }
    }
    libs
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase().trim_end_matches('\\')
        == b.to_string_lossy().to_lowercase().trim_end_matches('\\')
}

/// `userdata/<user>/config/localconfig.vdf` → apps → `<appid>` → `LastPlayed`.
fn read_last_played(root: &Path) -> HashMap<u32, u64> {
    let mut out = HashMap::new();
    let Ok(users) = std::fs::read_dir(root.join("userdata")) else { return out };
    for user in users.flatten() {
        let file = user.path().join("config").join("localconfig.vdf");
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        let doc = vdf::parse(&text);
        let Some(apps) = doc.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"])
        else {
            continue;
        };
        for (id, app) in apps.entries() {
            let (Ok(id), Some(t)) = (id.parse::<u32>(), app.str("LastPlayed")) else { continue };
            if let Ok(t) = t.parse::<u64>() {
                let e = out.entry(id).or_insert(0);
                *e = (*e).max(t);
            }
        }
    }
    out
}

/// Steam's local artwork cache for an app: (cover, hero, logo, icon, header).
pub struct LocalArt {
    pub cover: Option<PathBuf>,
    pub hero: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    pub header: Option<PathBuf>,
}

pub fn local_art(appid: u32) -> Option<LocalArt> {
    let root = steam_root(None)?;
    let dir = root.join("appcache").join("librarycache").join(appid.to_string());
    if !dir.is_dir() {
        return None;
    }
    let mut art = LocalArt { cover: None, hero: None, logo: None, icon: None, header: None };
    let mut visit = |p: PathBuf| {
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        match name.as_str() {
            "library_600x900.jpg" | "library_600x900_2x.jpg" => art.cover = art.cover.take().or(Some(p)),
            "library_hero.jpg" => art.hero = Some(p),
            "logo.png" => art.logo = Some(p),
            "header.jpg" => art.header = Some(p),
            // The icon is a 40-hex-char hash .jpg next to the other files.
            n if n.len() == 44 && n.ends_with(".jpg") => art.icon = Some(p),
            _ => {}
        }
    };
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            // Newer clients nest some files in hashed sub-folders.
            if let Ok(sub) = std::fs::read_dir(&p) {
                for e in sub.flatten() {
                    visit(e.path());
                }
            }
        } else {
            visit(p);
        }
    }
    Some(art)
}
