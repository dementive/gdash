//! Native PC games: every sub-folder of a configured root is one game.

use std::path::{Path, PathBuf};

use super::titles::{acronym, clean_title, lcs, normalize, slug};
use super::{Game, Launch, System};
use crate::config::Config;

const MAX_DEPTH: usize = 4;

/// Exe names (lowercase substrings) that are never the game.
const EXCLUDE_NAMES: &[&str] = &[
    "unins", "crash", "helper", "installer", "setup", "redist", "dxweb", "quicksfv", "vcredist",
    "dotnet", "directx", "glslang", "bssndrpt", "prereq", "physx", "bugreport", "reporter",
    "updater", "uninstall", "steamloader", "coldclient", "easyanticheat", "battleye", "cefprocess",
    "webhelper", "dxsetup", "oalinst", "touchup", "unitycrash", "notification_helper", "createdump",
    "steamservice", "subprocess", "ue4prereq", "ueprereq", "ffmpeg", "7z", "unrar", "launchpad_helper",
];

/// Directory names (lowercase) that never contain the game exe.
const EXCLUDE_DIRS: &[&str] = &[
    "_redist", "redist", "_commonredist", "commonredist", "directx", "vcredist", "dotnet", "support",
    "__installer", "installer", "installers", "prereqs", "prerequisites", "thirdparty", "easyanticheat",
    "battleye", "nodvd", "crack", "tools", "_cache", "cache", "logs", "mono", "monobleedingedge",
];

pub fn scan_root(root: &Path, cfg: &Config) -> Vec<Game> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut games = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let folder = entry.file_name().to_string_lossy().to_string();
        let title = clean_title(&folder);
        let id = format!("pc-{}", slug(&title));
        let ov = cfg.overrides.get(&id);

        let exe = match ov.and_then(|o| o.exe.clone()) {
            Some(p) if p.is_absolute() => Some(p),
            Some(p) => Some(dir.join(p)),
            None => pick_exe(&dir, &title),
        };
        let Some(exe) = exe else { continue };
        let cwd = exe.parent().map(Path::to_path_buf).unwrap_or_else(|| dir.clone());
        games.push(Game {
            id,
            title,
            system: System::Pc,
            launch: Launch::Exe {
                exe,
                args: ov.and_then(|o| o.args.clone()).unwrap_or_default(),
                cwd,
                watch_dir: Some(dir.clone()),
            },
            steam_appid: read_appid(&dir),
            art_name: None,
            last_played: None,
            controller: None,
            accent: None,
        });
    }
    games
}

fn read_appid(dir: &Path) -> Option<u32> {
    for name in ["steam_appid.txt", "steam_settings/steam_appid.txt"] {
        if let Ok(t) = std::fs::read_to_string(dir.join(name)) {
            let t = t.trim_matches(|c: char| c.is_whitespace() || c == '\0');
            if let Ok(id) = t.parse::<u32>() {
                if id != 480 {
                    // 480 is Valve's "Spacewar" test id, not the real game.
                    return Some(id);
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: PathBuf,
    pub depth: usize,
    pub size: u64,
}

pub fn pick_exe(dir: &Path, title: &str) -> Option<PathBuf> {
    let mut cands = Vec::new();
    collect(dir, 0, &mut cands);
    best_candidate(&cands, title).map(|c| c.path.clone())
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<Candidate>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if depth < MAX_DEPTH && !EXCLUDE_DIRS.contains(&name.as_str()) {
                collect(&p, depth + 1, out);
            }
        } else if name.ends_with(".exe") {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(Candidate { path: p, depth, size });
        }
    }
}

pub fn best_candidate<'a>(cands: &'a [Candidate], title: &str) -> Option<&'a Candidate> {
    cands
        .iter()
        .filter(|c| !excluded(c))
        .map(|c| (score(c, title), c))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| c)
}

fn stem_lower(c: &Candidate) -> String {
    c.path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase()
}

fn excluded(c: &Candidate) -> bool {
    let stem = stem_lower(c);
    EXCLUDE_NAMES.iter().any(|x| stem.contains(x))
}

pub fn score(c: &Candidate, title: &str) -> f32 {
    let stem = stem_lower(c);
    let ns = normalize(&stem);
    let nt = normalize(title);
    let acr = acronym(title);

    let mut s = 0.0;
    if !ns.is_empty() && ns == nt {
        s += 100.0;
    } else if !ns.is_empty() && (nt.contains(&ns) || ns.contains(&nt)) {
        s += 60.0;
    } else if acr.len() >= 2 && ns.starts_with(&acr) {
        s += 50.0;
    } else if !ns.is_empty() {
        s += 50.0 * lcs(&ns, &nt) as f32 / ns.len().max(nt.len()) as f32;
    }
    s -= c.depth as f32 * 10.0;
    s += ((c.size.max(1) as f32).log2() * 2.0).min(60.0);
    if stem.contains("launcher") {
        s -= 25.0;
    }
    if stem.contains("bootstrap") || stem.contains("config") || stem.contains("settings") {
        s -= 30.0;
    }
    if stem.contains("x86") || stem.ends_with("32") {
        s -= 10.0;
    }
    if stem.contains("test") || stem.contains("server") || stem.contains("editor") || stem.contains("modmanager") {
        s -= 30.0;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(path: &str, depth: usize, mb: u64) -> Candidate {
        Candidate { path: PathBuf::from(path), depth, size: mb * 1024 * 1024 }
    }

    fn pick<'a>(cands: &'a [Candidate], title: &str) -> &'a str {
        best_candidate(cands, title).unwrap().path.to_str().unwrap()
    }

    #[test]
    fn picks_main_exe() {
        let me = [
            c(r"G:\Games\Mass Effect\MassEffectLauncher.exe", 0, 5),
            c(r"G:\Games\Mass Effect\runme.exe", 0, 1),
            c(r"G:\Games\Mass Effect\unins000.exe", 0, 3),
            c(r"G:\Games\Mass Effect\Binaries\MassEffect.exe", 1, 20),
        ];
        assert_eq!(pick(&me, "Mass Effect"), r"G:\Games\Mass Effect\Binaries\MassEffect.exe");

        let aom = [
            c(r"G:\Games\AoM\AOEURLHelper.exe", 0, 1),
            c(r"G:\Games\AoM\AoEURLInstaller_Steam.exe", 0, 1),
            c(r"G:\Games\AoM\AoMRT_s.exe", 0, 150),
            c(r"G:\Games\AoM\BattleServer.exe", 0, 10),
        ];
        assert_eq!(pick(&aom, "Age of Mythology - Retold"), r"G:\Games\AoM\AoMRT_s.exe");

        let tw = [
            c(r"G:\Games\TW\Launcher.exe", 0, 2),
            c(r"G:\Games\TW\Pharaoh.exe", 0, 60),
            c(r"G:\Games\TW\assembly_kit\binaries\BOB.modder.x64.exe", 2, 30),
        ];
        assert_eq!(pick(&tw, "Total War - PHARAOH DYNASTIES"), r"G:\Games\TW\Pharaoh.exe");

        let sm = [
            c(r"G:\Games\SM\gamelaunchhelper.exe", 0, 1),
            c(r"G:\Games\SM\SpaceMarine.exe", 0, 40),
            c(r"G:\Games\SM\SpaceMarineBootstrapper.exe", 0, 2),
        ];
        assert_eq!(pick(&sm, "WH40K - Space Marine MCE"), r"G:\Games\SM\SpaceMarine.exe");

        let l4d = [
            c(r"G:\Games\L4D2\L4D2 Launcher.exe", 0, 1),
            c(r"G:\Games\L4D2\left4dead2.exe", 0, 1),
        ];
        assert_eq!(pick(&l4d, "Left 4 Dead 2"), r"G:\Games\L4D2\left4dead2.exe");

        let edf = [c(r"G:\Games\EDF\EDF5.exe", 0, 30), c(r"G:\Games\EDF\unins000.exe", 0, 2)];
        assert_eq!(pick(&edf, "Earth Defense Force 5"), r"G:\Games\EDF\EDF5.exe");

        let gc = [
            c(r"G:\Games\GC\GalCiv4.exe", 0, 80),
            c(r"G:\Games\GC\ModManager.exe", 0, 5),
            c(r"G:\Games\GC\SDCR64.exe", 0, 2),
            c(r"G:\Games\GC\StardockLauncher.exe", 0, 3),
        ];
        assert_eq!(pick(&gc, "Galactic Civilizations IV"), r"G:\Games\GC\GalCiv4.exe");

        let tab = [
            c(r"G:\Games\TAB\InstallSlimDX.exe", 0, 1),
            c(r"G:\Games\TAB\TestWindows.exe", 0, 1),
            c(r"G:\Games\TAB\TheyAreBillions.exe", 0, 20),
            c(r"G:\Games\TAB\TheyAreBillions_x86.exe", 0, 20),
        ];
        assert_eq!(pick(&tab, "They Are Billions"), r"G:\Games\TAB\TheyAreBillions.exe");
    }
}
