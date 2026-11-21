//! Original Xbox (xemu) and Xbox 360 (Xenia) game images.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::titles::{clean_title, slug};
use super::{Game, Launch, System};
use crate::config::Emulator;

fn emu_launch(emu: &Emulator, image: &Path) -> Launch {
    let path = image.to_string_lossy();
    Launch::Exe {
        exe: emu.exe.clone(),
        args: emu.args.iter().map(|a| a.replace("{path}", &path)).collect(),
        cwd: emu.exe.parent().map(Path::to_path_buf).unwrap_or_default(),
        watch_dir: None,
    }
}

/// Strips the image extension, keeping the No-Intro name for art lookups.
fn art_name(file_name: &str) -> String {
    let lower = file_name.to_ascii_lowercase();
    for ext in [".xiso.iso", ".iso", ".xex"] {
        if lower.ends_with(ext) {
            return file_name[..file_name.len() - ext.len()].to_string();
        }
    }
    file_name.to_string()
}

pub fn scan_xbox(emu: &Emulator) -> Vec<Game> {
    let mut games = Vec::new();
    for root in &emu.games {
        for iso in find_files(root, 2, &["iso", "xiso"]) {
            let name = iso.file_name().unwrap().to_string_lossy().to_string();
            let title = clean_title(&name);
            games.push(Game {
                id: format!("xbox-{}", slug(&title)),
                title,
                system: System::Xbox,
                launch: emu_launch(emu, &iso),
                steam_appid: None,
                art_name: Some(art_name(&name)),
                last_played: None,
                controller: None,
            });
        }
    }
    games
}

pub fn scan_x360(emu: &Emulator) -> Vec<Game> {
    let recent = read_xenia_recent(&emu.exe);
    let mut by_title: HashMap<String, Game> = HashMap::new();
    for root in &emu.games {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let image = if p.is_dir() {
                find_in_dir_360(&p)
            } else {
                let ext = p.extension().unwrap_or_default().to_string_lossy().to_lowercase();
                matches!(ext.as_str(), "iso" | "xex" | "zar").then(|| p.clone())
            };
            let Some(image) = image else { continue };

            let key = image.to_string_lossy().to_lowercase();
            let (title, last_played) = match recent.get(&key) {
                Some((t, lp)) => (t.clone(), Some(*lp)),
                None => (clean_title(&name), None),
            };
            let game = Game {
                id: format!("x360-{}", slug(&title)),
                title: title.clone(),
                system: System::Xbox360,
                launch: emu_launch(emu, &image),
                steam_appid: None,
                art_name: Some(art_name(&name)),
                last_played,
                controller: None,
            };
            // Prefer a disc image over loose files when the same title appears twice.
            let slot = slug(&title);
            let is_iso = image.extension().is_some_and(|e| e.eq_ignore_ascii_case("iso"));
            match by_title.get(&slot) {
                Some(_) if !is_iso => {}
                _ => {
                    by_title.insert(slot, game);
                }
            }
        }
    }
    by_title.into_values().collect()
}

/// A game folder can hold an ISO, a `default.xex`, or an STFS/GoD package
/// (`<TitleID>/000D0000/<file>` for XBLA, `<TitleID>/00007000/<file>` for Games on Demand).
fn find_in_dir_360(dir: &Path) -> Option<PathBuf> {
    if let Some(iso) = find_files(dir, 2, &["iso"]).into_iter().next() {
        return Some(iso);
    }
    let xex = dir.join("default.xex");
    if xex.is_file() {
        return Some(xex);
    }
    for title in std::fs::read_dir(dir).ok()?.flatten() {
        let tname = title.file_name().to_string_lossy().to_string();
        if tname.len() != 8 || !tname.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        for kind in ["000D0000", "00007000", "00004000"] {
            let pkg_dir = title.path().join(kind);
            if let Ok(files) = std::fs::read_dir(&pkg_dir) {
                if let Some(f) = files.flatten().map(|f| f.path()).find(|p| p.is_file()) {
                    return Some(f);
                }
            }
        }
    }
    None
}

fn find_files(dir: &Path, depth: usize, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if depth > 0 {
                out.extend(find_files(&p, depth - 1, exts));
            }
        } else if p
            .extension()
            .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
        {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Xenia's `recent.toml` has nice title names and last-run times keyed by path.
fn read_xenia_recent(exe: &Path) -> HashMap<String, (String, u64)> {
    let mut out = HashMap::new();
    let Some(dir) = exe.parent() else { return out };
    let Ok(text) = std::fs::read_to_string(dir.join("recent.toml")) else { return out };
    let Ok(doc) = text.parse::<toml::Table>() else { return out };
    for (_, v) in doc {
        let Some(t) = v.as_table() else { continue };
        let (Some(path), Some(name)) = (
            t.get("path").and_then(|v| v.as_str()),
            t.get("title_name").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let when = t.get("last_run_time").and_then(|v| v.as_integer()).unwrap_or(0) as u64;
        out.insert(path.to_lowercase(), (name.to_string(), when));
    }
    out
}
