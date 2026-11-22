//! `config.toml`: scan roots, emulator setup and per-game overrides.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Include installed Steam games.
    pub steam: bool,
    /// Override the Steam install folder (auto-detected from the registry when empty).
    pub steam_path: Option<PathBuf>,
    /// Folders whose sub-folders are each one native PC game.
    pub pc_roots: Vec<PathBuf>,
    /// Text file with one to-download page URL per line. Empty = look for
    /// `00_TO_DOWNLOAD.txt` in each `pc_roots` folder.
    pub wishlist_file: Option<PathBuf>,
    /// Original Xbox emulator (xemu).
    pub xemu: Option<Emulator>,
    /// Xbox 360 emulator (Xenia).
    pub xenia: Option<Emulator>,
    /// Game ids to hide from every blade.
    pub hidden: Vec<String>,
    /// Play UI sounds.
    pub sounds: bool,
    /// Improve artwork with on-device ML models: depth maps for the parallax background and
    /// super-resolution for low-resolution backgrounds. Models download once (~32 MB).
    pub enhance_art: bool,
    /// Per-game overrides keyed by game id (see `gdash --scan` for ids).
    pub overrides: BTreeMap<String, Override>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Emulator {
    pub exe: PathBuf,
    pub games: Vec<PathBuf>,
    /// Command-line arguments; `{path}` is replaced by the game image path.
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Override {
    pub title: Option<String>,
    /// For PC games: the exe to run (absolute, or relative to the game folder).
    pub exe: Option<PathBuf>,
    pub args: Option<Vec<String>>,
    /// Steam app id used to look up artwork.
    pub steam_appid: Option<u32>,
    /// Gamepad support: "full", "partial" or "none". Wins over anything gdash finds.
    pub controller: Option<String>,
    /// Signature colour for the glow ring and the Play pill, e.g. `accent = "#d65bd0"`.
    /// Wins over the colour lifted from the game's own art.
    pub accent: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            steam: true,
            steam_path: None,
            pc_roots: Vec::new(),
            wishlist_file: None,
            xemu: None,
            xenia: None,
            hidden: Vec::new(),
            sounds: true,
            enhance_art: true,
            overrides: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Loads the config, creating it with auto-detected paths on first run.
    pub fn load_or_create() -> Config {
        let file = paths::config_file();
        match std::fs::read_to_string(&file) {
            Ok(text) => match toml::from_str(&text) {
                Ok(cfg) => cfg,
                Err(err) => {
                    eprintln!("gdash: {} is invalid, using defaults: {err}", file.display());
                    Config::default()
                }
            },
            Err(_) => {
                let cfg = Config::detect();
                if let Err(err) = cfg.save() {
                    eprintln!("gdash: could not write {}: {err}", file.display());
                }
                cfg
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let file = paths::config_file();
        std::fs::create_dir_all(file.parent().unwrap())?;
        let body = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let header = "# gdash configuration. Delete this file to re-run auto-detection.\n\
                      # Run `gdash --scan` to list game ids for `hidden` and `[overrides.<id>]`.\n\
                      # The wishlist reads 00_TO_DOWNLOAD.txt from a pc_roots folder, or set wishlist_file.\n\n";
        std::fs::write(file, format!("{header}{body}"))
    }

    /// Looks on every drive for the usual layouts: `X:\Games`, `X:\xbox\xemu.exe`,
    /// `X:\xbox-360\xenia*.exe`.
    pub fn detect() -> Config {
        let mut cfg = Config::default();
        for letter in b'C'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", letter as char));
            if !root.exists() {
                continue;
            }
            let games = root.join("Games");
            if games.is_dir() {
                cfg.pc_roots.push(games);
            }
            if cfg.xemu.is_none() {
                let dir = root.join("xbox");
                let exe = dir.join("xemu.exe");
                if exe.is_file() {
                    cfg.xemu = Some(Emulator {
                        exe,
                        games: vec![dir.join("games")],
                        args: vec!["-full-screen".into(), "-dvd_path".into(), "{path}".into()],
                    });
                }
            }
            if cfg.xenia.is_none() {
                let dir = root.join("xbox-360");
                if let Some(exe) = find_xenia(&dir) {
                    cfg.xenia = Some(Emulator {
                        exe,
                        games: vec![dir.join("games")],
                        args: vec!["{path}".into(), "--fullscreen".into()],
                    });
                }
            }
        }
        cfg
    }
}

fn find_xenia(dir: &Path) -> Option<PathBuf> {
    ["xenia_canary.exe", "xenia.exe"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}
