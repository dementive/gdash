//! Games the user plans to download, read from a plain URL list.
//!
//! The list is one page URL per line (blank lines, `#` and `//` comments are ignored).
//! A line may also be `Title | URL` (or `Title<TAB>URL`) to override the title derived
//! from the URL slug. Entries whose title matches an installed game are dropped.

use std::collections::HashSet;
use std::path::PathBuf;

use super::titles::{normalize, slug};
use super::{Game, Launch, System};
use crate::config::Config;

const DEFAULT_FILE: &str = "00_TO_DOWNLOAD.txt";

/// The host of a page URL, for display.
pub fn source_host(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .filter(|s| !s.is_empty())
        .unwrap_or(url)
}

/// Reads every configured list file and turns the entries into wishlist games.
pub fn load(cfg: &Config, installed: &[Game]) -> Vec<Game> {
    let mut items = Vec::new();
    for path in list_files(cfg) {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        items.extend(parse(&text));
    }
    from_items(items, installed)
}

fn list_files(cfg: &Config) -> Vec<PathBuf> {
    if let Some(path) = &cfg.wishlist_file {
        return if path.is_dir() { vec![path.join(DEFAULT_FILE)] } else { vec![path.clone()] };
    }
    cfg.pc_roots
        .iter()
        .map(|root| root.join(DEFAULT_FILE))
        .filter(|path| path.is_file())
        .collect()
}

/// Parses `Title | URL` / URL lines. Invalid lines are skipped.
fn parse(text: &str) -> Vec<(String, String)> {
    let mut items = Vec::new();
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let Some(start) = lower.find("http://").or_else(|| lower.find("https://")) else { continue };
        let url = line[start..].trim().to_string();
        let manual = line[..start].trim().trim_end_matches('|').trim();
        let title = if manual.is_empty() { title_from_url(&url) } else { manual.to_string() };
        if !title.is_empty() && !slug(&title).is_empty() {
            items.push((title, url));
        }
    }
    items
}

fn from_items(items: Vec<(String, String)>, installed: &[Game]) -> Vec<Game> {
    let installed: HashSet<String> = installed
        .iter()
        .map(|g| normalize(&g.title))
        .filter(|s| !s.is_empty())
        .collect();
    let mut seen_titles = HashSet::new();
    let mut seen_urls = HashSet::new();
    let mut seen_ids = HashSet::new();
    let mut games = Vec::new();
    for (title, url) in items {
        let key = normalize(&title);
        let url_key = url.trim_end_matches('/').to_ascii_lowercase();
        if key.is_empty()
            || installed.contains(&key)
            || !seen_titles.insert(key)
            || !seen_urls.insert(url_key)
        {
            continue;
        }
        let base = format!("wish-{}", slug(&title));
        let mut id = base.clone();
        let mut n = 2;
        while !seen_ids.insert(id.clone()) {
            id = format!("{base}-{n}");
            n += 1;
        }
        games.push(Game {
            id,
            title,
            system: System::Wishlist,
            launch: Launch::Url { url },
            steam_appid: None,
            art_name: None,
            last_played: None,
            controller: None,
            accent: None,
        });
    }
    games.sort_by_key(|g| g.sort_key());
    games
}

fn title_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/');
    let slug = path.rsplit('/').next().unwrap_or("");
    title_from_slug(&slug.replace("%20", " "))
}

/// Turns a site slug into something readable: `a-total-war-saga-troy` → `A Total War Saga Troy`.
fn title_from_slug(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let mut tokens: Vec<&str> = lower.split(['-', '_']).filter(|t| !t.is_empty()).collect();

    // Site noise: `game-free-download-1o` ends the title at "free".
    if let Some(i) = tokens.windows(2).position(|w| w[0] == "free" && w[1] == "download") {
        tokens.truncate(i);
    }

    // Version suffixes: `...-v1-3-3-dlcs-ultra-hd-textures`. Keep a lone `v2` in
    // "Sniper Elite V2" — only cut when a version-ish token follows.
    if let Some(i) = tokens.iter().enumerate().position(|(i, t)| {
        let is_version = t.len() >= 2
            && t.starts_with('v')
            && t[1..].chars().all(|c| c.is_ascii_digit());
        if !is_version {
            return false;
        }
        tokens.get(i + 1).is_some_and(|next| {
            next.starts_with(|c: char| c.is_ascii_digit())
                || matches!(*next, "dlc" | "dlcs" | "multiplayer" | "textures" | "ultra" | "hd")
        })
    }) {
        tokens.truncate(i);
    }

    // Trailing release/site noise.
    const TRAILING: &[&str] = &[
        "dlc", "dlcs", "multiplayer", "ultra", "hd", "textures", "pc", "repack", "repacks",
    ];
    while tokens.last().is_some_and(|t| TRAILING.contains(t)) {
        tokens.pop();
    }

    if tokens.is_empty() {
        return String::new();
    }

    const STALKER: [&str; 7] = ["s", "t", "a", "l", "k", "e", "r"];
    let mut words = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i..].starts_with(&STALKER) {
            words.push("S.T.A.L.K.E.R.".to_string());
            i += STALKER.len();
        } else if tokens[i] == "p" && tokens.get(i + 1) == Some(&"i") {
            words.push("P.I.".to_string());
            i += 2;
        } else {
            words.push(title_word(tokens[i], words.is_empty()));
            i += 1;
        }
    }
    words.join(" ")
}

fn title_word(word: &str, first: bool) -> String {
    const SMALL: &[&str] = &[
        "a", "an", "and", "as", "at", "but", "by", "for", "from", "in", "into", "of", "on",
        "onto", "or", "over", "per", "the", "to", "up", "via", "vs", "with",
    ];
    const ROMAN: &[&str] = &[
        "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv",
    ];
    let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '\'' && c != '&');
    if cleaned.is_empty() {
        return String::new();
    }
    if cleaned == "40000" {
        return "40,000".into();
    }
    let lower = cleaned.to_ascii_lowercase();
    let possessive = match lower.as_str() {
        "clancys" => Some("Clancy's"),
        "meiers" => Some("Meier's"),
        "bards" => Some("Bard's"),
        "necromancers" => Some("Necromancer's"),
        _ => None,
    };
    if let Some(name) = possessive {
        return name.into();
    }
    if ROMAN.contains(&lower.as_str()) {
        return lower.to_ascii_uppercase();
    }
    if !first && SMALL.contains(&lower.as_str()) {
        return lower;
    }
    let mut chars = lower.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(title: &str) -> Game {
        Game {
            id: format!("pc-{}", slug(title)),
            title: title.into(),
            system: System::Pc,
            launch: Launch::Exe {
                exe: PathBuf::from(r"C:\game.exe"),
                args: Vec::new(),
                cwd: PathBuf::from(r"C:\"),
                watch_dir: None,
            },
            steam_appid: None,
            art_name: None,
            last_played: None,
            controller: None,
            accent: None,
        }
    }

    #[test]
    fn parses_url_lists_with_blank_lines() {
        let text = "\nhttps://fitgirl-repacks.site/age-of-wonders-4/\n\n\
                    https://steamrip.com/xenonauts-2-free-download-1v/\n";
        let items = parse(text);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, "Age of Wonders 4");
        assert_eq!(items[1].0, "Xenonauts 2");
    }

    #[test]
    fn cleans_slug_titles() {
        assert_eq!(title_from_slug("age-of-wonders-4"), "Age of Wonders 4");
        assert_eq!(title_from_slug("diablo-ii-resurrected-pc"), "Diablo II Resurrected");
        assert_eq!(
            title_from_slug("s-t-a-l-k-e-r-2-heart-of-chornobyl"),
            "S.T.A.L.K.E.R. 2 Heart of Chornobyl"
        );
        assert_eq!(
            title_from_slug("warhammer-40000-space-marine-collection-v1-0-165-dlcs"),
            "Warhammer 40,000 Space Marine Collection"
        );
        assert_eq!(
            title_from_slug("far-cry-primal-apex-edition-v1-3-3-dlcs-ultra-hd-textures"),
            "Far Cry Primal Apex Edition"
        );
        assert_eq!(
            title_from_slug("star-wars-empire-at-war-gold-pack-free-download-1o"),
            "Star Wars Empire at War Gold Pack"
        );
        assert_eq!(title_from_slug("sniper-elite-v2-v1-13-5-dlcs"), "Sniper Elite V2");
        assert_eq!(title_from_slug("sid-meiers-civilization-beyond-earth-2-dlc"),
            "Sid Meier's Civilization Beyond Earth 2");
        assert_eq!(title_from_slug("tom-clancys-splinter-cell-blacklist-digital-deluxe-edition-v1-03-2-dlcs"),
            "Tom Clancy's Splinter Cell Blacklist Digital Deluxe Edition");
        assert_eq!(title_from_slug("mouse-p-i-for-hire"), "Mouse P.I. for Hire");
    }

    #[test]
    fn supports_manual_titles() {
        let items = parse("My Game | https://example.com/some-slug/\nhttps://example.com/another/\n");
        assert_eq!(items[0].0, "My Game");
        assert_eq!(items[1].0, "Another");
    }

    #[test]
    fn filters_installed_and_duplicates() {
        let items = vec![
            ("Mass Effect".into(), "https://example.com/mass-effect".into()),
            ("Age of Wonders 4".into(), "https://example.com/aow4".into()),
            ("Age of Wonders IV".into(), "https://example.com/aow4-again".into()),
            ("Age of Wonders 4".into(), "https://example.com/aow4-third".into()),
        ];
        let games = from_items(items, &[installed("Mass Effect")]);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title, "Age of Wonders 4");
        assert_eq!(games[0].id, "wish-age-of-wonders-4");
    }

    #[test]
    fn reads_a_configured_file() {
        let dir = std::env::temp_dir().join(format!("gdash-wishlist-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("list.txt");
        std::fs::write(&file, "https://example.com/silica/\n").unwrap();
        let cfg = Config { wishlist_file: Some(file), ..Default::default() };
        let games = load(&cfg, &[]);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title, "Silica");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
