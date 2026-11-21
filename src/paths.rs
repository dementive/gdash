//! Where gdash keeps its config, caches and state.

use std::path::PathBuf;

fn dirs() -> directories::ProjectDirs {
    directories::ProjectDirs::from("", "", "gdash").expect("no home directory")
}

/// `%APPDATA%\gdash\config` — user-editable config.
pub fn config_dir() -> PathBuf {
    dirs().config_dir().to_path_buf()
}

/// `%LOCALAPPDATA%\gdash\data` — caches (library, art) and play state.
pub fn data_dir() -> PathBuf {
    dirs().data_local_dir().to_path_buf()
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn library_cache_file() -> PathBuf {
    data_dir().join("library.json")
}

pub fn state_file() -> PathBuf {
    data_dir().join("state.json")
}

pub fn art_dir() -> PathBuf {
    data_dir().join("art")
}
