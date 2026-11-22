//! gdash — a blade-style fullscreen game launcher.

// A Windows GUI program: no console window, and games it starts don't inherit one.
#![windows_subsystem = "windows"]

mod art;
mod config;
mod enhance;
mod library;
mod paths;
mod ui;
mod win;

use std::sync::mpsc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    // Command-line modes print results: borrow the terminal they were started from.
    if ["--help", "-h", "--scan", "--fetch-art", "--refresh-art", "--enhance-art"].iter().any(|f| has(f)) {
        win::attach_parent_console();
    }

    if has("--help") || has("-h") {
        println!(
            "gdash [--scan] [--fetch-art] [--refresh-art] [--enhance-art] [--windowed] [--screenshot <file.png>] [--nav <UDLRQEABYGKPXVW>]\n\
             config: {}\n data: {}",
            paths::config_file().display(),
            paths::data_dir().display()
        );
        return;
    }

    let cfg = config::Config::load_or_create();

    if has("--scan") {
        let games = library::scan_all(&cfg);
        library::save_cache(&games);
        println!("{}", serde_json::to_string_pretty(&games).unwrap());
        let count = |s| games.iter().filter(|g| g.system == s).count();
        let pad = |l| games.iter().filter(|g| g.controller == Some(l)).count();
        let wishlist = library::wishlist::load(&cfg, &games);
        eprintln!(
            "found {} games: steam {}, pc {}, xbox {}, x360 {}; wishlist {} not installed",
            games.len(),
            count(library::System::Steam),
            count(library::System::Pc),
            count(library::System::Xbox),
            count(library::System::Xbox360),
            wishlist.len()
        );
        eprintln!(
            "controller support: {} full, {} partial, {} none, {} unknown (run --fetch-art to look up unknowns)",
            pad(library::ControllerSupport::Full),
            pad(library::ControllerSupport::Partial),
            pad(library::ControllerSupport::None),
            games.iter().filter(|g| g.controller.is_none()).count()
        );
        return;
    }

    if has("--fetch-art") || has("--refresh-art") {
        if has("--refresh-art") {
            art::clear(false);
            library::controller::clear_unknown();
        }
        let games = library::scan_all(&cfg);
        // Controller lookups first: they're quick, cached, and printed in the summary.
        let games = library::controller::refresh(games);
        library::save_cache(&games);
        let pad = |l| games.iter().filter(|g| g.controller == Some(l)).count();
        eprintln!(
            "controller support: {} full, {} partial, {} none, {} unknown",
            pad(library::ControllerSupport::Full),
            pad(library::ControllerSupport::Partial),
            pad(library::ControllerSupport::None),
            games.iter().filter(|g| g.controller.is_none()).count()
        );
        // `fetch_all` pops from the end; reversed wishlist first means A→Z before the rest.
        let wishlist = library::wishlist::load(&cfg, &games);
        let mut art_games = games;
        art_games.extend(wishlist.into_iter().rev());
        let total = art_games.len();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || art::fetch_all(art_games, 6, tx));
        for (i, r) in rx.iter().enumerate() {
            let marks: String = r
                .found
                .iter()
                .map(|(k, ok)| format!("{:?}:{} ", k, if *ok { "Y" } else { "-" }))
                .collect();
            println!("[{:>2}/{total}] {:<48} {marks}", i + 1, r.title);
        }
        let _ = worker.join();
        return;
    }

    if has("--enhance-art") {
        let games = library::load_cache().unwrap_or_else(|| library::scan_all(&cfg));
        // Wishlist heroes show too (the grid backdrop) — they just aren't installed.
        let mut games = games;
        games.extend(library::wishlist::load(&cfg, &games));
        enhance::run_cli(&games);
        return;
    }

    let value_of = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    ui::run(
        ui::Options {
            windowed: has("--windowed"),
            screenshot: value_of("--screenshot").map(std::path::PathBuf::from),
            script: value_of("--nav").unwrap_or_default(),
        },
        cfg,
    );
}
