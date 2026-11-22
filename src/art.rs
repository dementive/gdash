//! Artwork: cover (box art), hero (wide background), logo and icon per game.
//!
//! Lookup order: user-supplied file in the cache folder → Steam's local library cache →
//! Steam CDN (by app id or store search) → libretro-thumbnails (Xbox/360 box art) →
//! the exe's own icon. Results are resized once and cached under
//! `%LOCALAPPDATA%\gdash\data\art\<game-id>\`. Misses leave a `<kind>.none` marker so we
//! don't hit the network every launch (`--refresh-art` clears them).
//!
//! A game can also carry alternate heroes (`hero2` / `hero3`, fetched from the store's
//! screenshots or dropped in by hand): the showcase rotates through them while the game
//! is selected, so sitting still still changes the picture.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use image::{DynamicImage, ImageFormat};
use serde::Deserialize;

use crate::library::titles::{clean_title, libretro_name, similarity, strip_groups};
use crate::library::{Game, Launch, System, steam};
use crate::paths;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ArtKind {
    Cover,
    Hero,
    Logo,
    Icon,
}

impl ArtKind {
    pub const ALL: [ArtKind; 4] = [ArtKind::Cover, ArtKind::Hero, ArtKind::Logo, ArtKind::Icon];

    fn stem(self) -> &'static str {
        match self {
            ArtKind::Cover => "cover",
            ArtKind::Hero => "hero",
            ArtKind::Logo => "logo",
            ArtKind::Icon => "icon",
        }
    }

    /// Max (width, height) stored on disk.
    fn max_size(self) -> (u32, u32) {
        match self {
            ArtKind::Cover => (600, 900),
            // Full 4K: heroes fill the screen, so every pixel shows on a big display.
            ArtKind::Hero => (3840, 2160),
            ArtKind::Logo => (800, 400),
            ArtKind::Icon => (96, 96),
        }
    }

    /// Logos and icons need alpha → PNG; photos → JPEG.
    fn format(self) -> ImageFormat {
        match self {
            ArtKind::Cover | ArtKind::Hero => ImageFormat::Jpeg,
            ArtKind::Logo | ArtKind::Icon => ImageFormat::Png,
        }
    }
}

pub fn game_dir(id: &str) -> PathBuf {
    paths::art_dir().join(id)
}

/// The cached file for `kind`, if any (any image extension, so users can drop in their own).
pub fn cached(id: &str, kind: ArtKind) -> Option<PathBuf> {
    let dir = game_dir(id);
    ["png", "jpg", "jpeg", "webp"]
        .iter()
        .map(|ext| dir.join(format!("{}.{ext}", kind.stem())))
        .find(|p| p.is_file())
}

/// The cached file for `kind`, `variant` 0..3 — heroes rotate: `hero` / `hero2` / `hero3`.
pub fn cached_at(id: &str, kind: ArtKind, variant: u32) -> Option<PathBuf> {
    match kind {
        ArtKind::Hero => cached_hero_at(id, variant),
        _ if variant == 0 => cached(id, kind),
        _ => None,
    }
}

/// Alternate heroes: the showcase quietly cross-fades between them while a game is
/// selected (see view::update_background). Fetched from the store's screenshots, or
/// dropped in by hand alongside the rest of a game's art. `v` is the variant index:
/// 0 = `hero`, 1 = `hero2`, 2 = `hero3`.
pub fn hero_stem(v: u32) -> String {
    if v == 0 { "hero".into() } else { format!("hero{}", v + 1) }
}

pub fn cached_hero_at(id: &str, v: u32) -> Option<PathBuf> {
    let dir = game_dir(id);
    ["png", "jpg", "jpeg", "webp"]
        .iter()
        .map(|ext| dir.join(format!("{}.{ext}", hero_stem(v))))
        .find(|p| p.is_file())
}

pub fn hero_at_missing(id: &str, v: u32) -> bool {
    game_dir(id).join(format!("{}.none", hero_stem(v))).is_file()
}

/// How many hero variants a game has on disk (0 when it has no hero at all).
pub fn hero_count(id: &str) -> usize {
    (0..3).filter(|&v| cached_hero_at(id, v).is_some()).count()
}

fn marked_missing(id: &str, kind: ArtKind) -> bool {
    game_dir(id).join(format!("{}.none", kind.stem())).is_file()
}

/// Removes miss markers (and, with `all`, every cached image) so art is fetched again.
pub fn clear(all: bool) {
    let Ok(dirs) = std::fs::read_dir(paths::art_dir()) else { return };
    for d in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(d.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if name.ends_with(".none") || (all && !name.starts_with("user")) {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct FetchReport {
    pub id: String,
    pub title: String,
    pub found: Vec<(ArtKind, bool)>,
}

/// Fetches art for many games on a few worker threads, sending a report per game.
pub fn fetch_all(games: Vec<Game>, workers: usize, tx: mpsc::Sender<FetchReport>) {
    let queue = Arc::new(Mutex::new(games));
    let mut handles = Vec::new();
    for _ in 0..workers.max(1) {
        let queue = queue.clone();
        let tx = tx.clone();
        handles.push(std::thread::spawn(move || {
            let fetcher = Fetcher::new();
            loop {
                let Some(game) = queue.lock().unwrap().pop() else { break };
                let report = fetcher.fetch(&game);
                if tx.send(report).is_err() {
                    break;
                }
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
}

pub struct Fetcher {
    agent: ureq::Agent,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher {
    pub fn new() -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent("gdash/0.1 (game launcher)")
            .build()
            .into();
        Self { agent }
    }

    /// Fills in whatever art is missing for `game`, then fetches hero alternates. Returns
    /// which of the primary kinds were found.
    pub fn fetch(&self, game: &Game) -> FetchReport {
        let dir = game_dir(&game.id);
        let _ = std::fs::create_dir_all(&dir);
        let mut missing: Vec<ArtKind> = ArtKind::ALL
            .into_iter()
            .filter(|&k| cached(&game.id, k).is_none() && !marked_missing(&game.id, k))
            .collect();
        // Heroes cached at low resolution (older versions shrank them) get one upgrade attempt.
        let hq_marker = dir.join("hero.hq");
        let old_hero_w = cached(&game.id, ArtKind::Hero)
            .filter(|_| !hq_marker.exists())
            .and_then(|p| image::image_dimensions(p).ok())
            .map(|(w, _)| w)
            .filter(|&w| w < HQ_HERO_W);
        if old_hero_w.is_some() {
            missing.push(ArtKind::Hero);
        }

        // The app id `gather` resolved, if it needed one (reused for hero alternates).
        let mut resolved = None;
        if !missing.is_empty() {
            let mut sources = Sources::default();
            resolved = self.gather(game, &missing, &mut sources);
            for kind in missing {
                if kind == ArtKind::Hero {
                    if let Some(old_w) = old_hero_w {
                        if let Some(img) = sources.get(kind).filter(|i| i.width() > old_w) {
                            if let Some(old) = cached(&game.id, kind) {
                                let _ = std::fs::remove_file(old);
                            }
                            let _ = save(&dir, kind, img);
                        }
                        let _ = std::fs::write(&hq_marker, b"");
                        continue;
                    }
                }
                let ok = match sources.get(kind) {
                    Some(img) => save(&dir, kind, img).is_ok(),
                    None => false,
                };
                if !ok {
                    let _ = std::fs::write(dir.join(format!("{}.none", kind.stem())), b"");
                }
            }
        }

        // Hero variety: a couple of the store's screenshots become quiet alternates
        // (hero2 / hero3) that the showcase rotates through while the game is selected.
        // Only for games with a hero and a Steam-resolvable id; misses are marked like
        // any other kind, so `--refresh-art` clears them along with the rest.
        let alts: Vec<u32> = [1, 2]
            .into_iter()
            .filter(|&v| cached_hero_at(&game.id, v).is_none() && !hero_at_missing(&game.id, v))
            .collect();
        if !alts.is_empty() && cached(&game.id, ArtKind::Hero).is_some() {
            if let Some(appid) = resolved.or_else(|| self.resolve_appid(game)) {
                let shots = self.store_screenshots(appid).unwrap_or_default();
                // The first screenshot is usually the hero's own key art; skip it when
                // there's plenty to choose from.
                let pool: Vec<&String> = if shots.len() > 3 { shots[1..].iter().collect() } else { shots.iter().collect() };
                for (i, &v) in alts.iter().enumerate() {
                    let ok = pool
                        .get(i)
                        .and_then(|u| self.get_image(u))
                        .filter(|img| img.width() >= 1280 && img.height() >= 720)
                        .map(|img| save_jpg(&dir, &hero_stem(v), &img, 3840, 2160, 95).is_ok())
                        .unwrap_or(false);
                    if !ok {
                        let _ = std::fs::write(dir.join(format!("{}.none", hero_stem(v))), b"");
                    }
                }
            }
        }

        FetchReport {
            id: game.id.clone(),
            title: game.title.clone(),
            found: ArtKind::ALL.iter().map(|&k| (k, cached(&game.id, k).is_some())).collect(),
        }
    }

    /// The Steam app id to use for `game`'s art: its own id when the store name matches
    /// (ids from steam_appid.txt next to a PC game are often a DLC or test id), else the
    /// best store search hit.
    fn resolve_appid(&self, game: &Game) -> Option<u32> {
        let known = game.steam_appid.filter(|&id| {
            game.system == System::Steam || self.app_name(id).is_some_and(|n| titles_match(&game.title, &n))
        });
        known.or_else(|| {
            let strict = game.system != System::Pc;
            self.store_search(&game.title, strict)
        })
    }

    fn gather(&self, game: &Game, missing: &[ArtKind], src: &mut Sources) -> Option<u32> {
        let wants = |k: ArtKind| missing.contains(&k);
        let mut local_hero = None;

        // 1. Steam's local cache.
        if let Some(appid) = game.steam_appid {
            if let Some(local) = steam::local_art(appid) {
                src.cover = local.cover.as_deref().and_then(load_file);
                src.hero = local.hero.as_deref().and_then(load_file);
                // Steam often caches only the 1920-wide hero; the store has a 3840 one.
                if src.hero.as_ref().is_some_and(|h| h.width() < HQ_HERO_W) {
                    local_hero = src.hero.take();
                }
                src.logo = local.logo.as_deref().and_then(load_file);
                src.icon = local.icon.as_deref().and_then(load_file);
            }
        }

        // 2. Exe icon for PC games.
        if wants(ArtKind::Icon) && src.icon.is_none() {
            if let Launch::Exe { exe, .. } = &game.launch {
                if game.system == System::Pc {
                    if let Some((w, h, px)) = crate::win::exe_icon_rgba(exe, 256) {
                        src.icon = image::RgbaImage::from_raw(w, h, px).map(DynamicImage::ImageRgba8);
                    }
                }
            }
        }

        // 3. libretro box art for emulated systems.
        if matches!(game.system, System::Xbox | System::Xbox360) {
            if wants(ArtKind::Cover) && src.cover.is_none() {
                src.cover = self.libretro(game, "Named_Boxarts");
            }
        }

        // 4. Steam CDN by app id (known or searched).
        let need_steam = [ArtKind::Cover, ArtKind::Hero, ArtKind::Logo]
            .iter()
            .any(|&k| wants(k) && src.get(k).is_none());
        let mut resolved = None;
        if need_steam {
            if let Some(appid) = self.resolve_appid(game) {
                // Newer apps keep art under hashed paths that only the store API knows.
                let assets = self.store_assets(appid);
                let pick = |names: &[&str]| -> Vec<String> {
                    names.iter().filter_map(|n| assets.as_ref().and_then(|a| a.url(n))).collect()
                };
                let try_urls = |urls: Vec<String>, legacy: &[&str]| -> Option<DynamicImage> {
                    urls.iter()
                        .find_map(|u| self.get_image(u))
                        .or_else(|| legacy.iter().find_map(|f| self.cdn(appid, f)))
                };
                if wants(ArtKind::Cover) && src.cover.is_none() {
                    src.cover = try_urls(
                        pick(&["library_capsule_2x", "library_capsule"]),
                        &["library_600x900_2x.jpg", "library_600x900.jpg"],
                    );
                }
                if wants(ArtKind::Hero) && src.hero.is_none() {
                    src.hero = try_urls(
                        pick(&["library_hero_2x", "library_hero", "page_background", "raw_page_background"]),
                        &["library_hero_2x.jpg", "library_hero.jpg"],
                    );
                }
                if wants(ArtKind::Logo) && src.logo.is_none() {
                    let mut urls = Vec::new();
                    if let Some(a) = &assets {
                        urls.extend(a.url_file("logo.png"));
                        // The logo usually sits next to the hashed hero.
                        if let Some(dir) = a.library_hero.as_deref().and_then(|h| h.rsplit_once('/')) {
                            urls.extend(a.url_file(&format!("{}/logo.png", dir.0)));
                        }
                    }
                    src.logo = try_urls(urls, &["logo.png"]);
                }
                if wants(ArtKind::Icon) && src.icon.is_none() {
                    if let Some(icon) = assets.as_ref().and_then(|a| a.community_icon.clone()) {
                        src.icon = self.get_image(&format!(
                            "https://shared.cloudflare.steamstatic.com/steamcommunity/public/images/apps/{appid}/{icon}.jpg"
                        ));
                    }
                }
                resolved = Some(appid);
            }
        }

        if src.hero.is_none() {
            src.hero = local_hero;
        }

        // 5. Emulated games without a Steam twin: use an in-game snap as the hero.
        if matches!(game.system, System::Xbox | System::Xbox360)
            && wants(ArtKind::Hero)
            && src.hero.is_none()
        {
            src.hero = self
                .libretro(game, "Named_Snaps")
                .or_else(|| self.libretro(game, "Named_Titles"));
        }

        resolved
    }

    /// The store's full-size screenshots for an app id, via the appdetails
    /// `screenshots` filter (in-game shots — hero-grade art, no extra page fetch).
    fn store_screenshots(&self, appid: u32) -> Option<Vec<String>> {
        let url = format!(
            "https://store.steampowered.com/api/appdetails?appids={appid}&cc=US&l=english&filters=screenshots"
        );
        let mut resp = self.agent.get(&url).call().ok()?;
        let text = resp.body_mut().read_to_string().ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let shots = v.get(appid.to_string())?.get("data")?.get("screenshots")?.as_array()?;
        Some(shots.iter().filter_map(|s| s.get("path_full")?.as_str().map(str::to_string)).collect())
    }

    fn get_image(&self, url: &str) -> Option<DynamicImage> {
        let mut resp = self.agent.get(url).call().ok()?;
        let bytes = resp.body_mut().with_config().limit(25 * 1024 * 1024).read_to_vec().ok()?;
        image::load_from_memory(&bytes).ok()
    }

    fn cdn(&self, appid: u32, file: &str) -> Option<DynamicImage> {
        [
            format!("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{appid}/{file}"),
            format!("https://cdn.akamai.steamstatic.com/steam/apps/{appid}/{file}"),
        ]
        .iter()
        .find_map(|u| self.get_image(u))
    }

    fn libretro(&self, game: &Game, folder: &str) -> Option<DynamicImage> {
        let repo = match game.system {
            System::Xbox => "Microsoft_-_Xbox",
            System::Xbox360 => "Microsoft_-_Xbox_360",
            _ => return None,
        };
        let raw = game.art_name.clone().unwrap_or_else(|| game.title.clone());
        libretro_candidates(&raw).into_iter().find_map(|name| {
            let url = format!(
                "https://raw.githubusercontent.com/libretro-thumbnails/{repo}/master/{folder}/{}.png",
                url_encode(&libretro_name(&name))
            );
            self.get_image(&url)
        })
    }

    /// Asset file names (with their hashed folders) from the store browse API.
    fn store_assets(&self, appid: u32) -> Option<StoreAssets> {
        let input = format!(
            r#"{{"ids":[{{"appid":{appid}}}],"context":{{"language":"english","country_code":"US"}},"data_request":{{"include_assets":true}}}}"#
        );
        let url = format!(
            "https://api.steampowered.com/IStoreBrowseService/GetItems/v1?input_json={}",
            url_encode(&input)
        );
        let mut resp = self.agent.get(&url).call().ok()?;
        let text = resp.body_mut().read_to_string().ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let assets = v.pointer("/response/store_items/0/assets")?.clone();
        serde_json::from_value(assets).ok()
    }

    /// The store page's name, controller-support tag and category ids for an app id.
    pub fn app_details(&self, appid: u32) -> Option<AppDetails> {
        let url = format!("https://store.steampowered.com/api/appdetails?appids={appid}&cc=US&l=english");
        let mut resp = self.agent.get(&url).call().ok()?;
        let text = resp.body_mut().read_to_string().ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let d = v.get(appid.to_string())?.get("data")?;
        let name = d.get("name")?.as_str()?.to_string();
        let controller_tag = d.get("controller_support").and_then(|t| t.as_str()).map(str::to_string);
        let categories = d
            .get("categories")
            .and_then(|c| c.as_array())
            .map(|a| a.iter().filter_map(|x| x.get("id").and_then(|i| i.as_u64()).map(|i| i as u32)).collect())
            .unwrap_or_default();
        Some(AppDetails { name, controller_tag, categories })
    }

    /// The store name of an app id (None for unknown ids).
    pub(crate) fn app_name(&self, appid: u32) -> Option<String> {
        let url = format!("https://store.steampowered.com/api/appdetails?appids={appid}&filters=basic");
        let mut resp = self.agent.get(&url).call().ok()?;
        let text = resp.body_mut().read_to_string().ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        v.get(appid.to_string())?.get("data")?.get("name")?.as_str().map(str::to_string)
    }

    /// Finds a Steam app id by name. `strict` demands a near-exact title match.
    pub fn store_search(&self, title: &str, strict: bool) -> Option<u32> {
        #[derive(Deserialize)]
        struct Item {
            id: u32,
            name: String,
        }
        #[derive(Deserialize)]
        struct Resp {
            items: Vec<Item>,
        }
        let mut best: Option<(f32, u32)> = None;
        for q in search_queries(title) {
            let url = format!(
                "https://store.steampowered.com/api/storesearch/?term={}&l=english&cc=US",
                url_encode(&q)
            );
            let Ok(mut resp) = self.agent.get(&url).call() else { continue };
            let Ok(text) = resp.body_mut().read_to_string() else { continue };
            let Ok(parsed) = serde_json::from_str::<Resp>(&text) else { continue };
            for item in parsed.items {
                let s = match_score(title, &item.name, strict);
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, item.id));
                }
            }
            if best.is_some_and(|(s, _)| s >= 0.99) {
                break;
            }
        }
        let threshold = if strict { 0.9 } else { 0.8 };
        best.filter(|(s, _)| *s >= threshold).map(|(_, id)| id)
    }
}

/// The bits of a Steam store page gdash cares about beyond artwork.
pub struct AppDetails {
    pub name: String,
    /// `controller_support`: "full" / "partial", when the page states it.
    pub controller_tag: Option<String>,
    /// Store category ids (28 = "Full controller support", 18 = "Partial Controller
    /// Support"). Many pages state support only here — the tag field is missing for
    /// them — and `filters=basic` drops the array, so this needs the full page.
    pub categories: Vec<u32>,
}

#[derive(Deserialize, Default, Debug)]
struct StoreAssets {
    asset_url_format: Option<String>,
    library_capsule: Option<String>,
    library_capsule_2x: Option<String>,
    library_hero: Option<String>,
    library_hero_2x: Option<String>,
    page_background: Option<String>,
    raw_page_background: Option<String>,
    community_icon: Option<String>,
}

impl StoreAssets {
    fn url(&self, field: &str) -> Option<String> {
        let file = match field {
            "library_capsule" => self.library_capsule.as_ref(),
            "library_capsule_2x" => self.library_capsule_2x.as_ref(),
            "library_hero" => self.library_hero.as_ref(),
            "library_hero_2x" => self.library_hero_2x.as_ref(),
            "page_background" => self.page_background.as_ref(),
            "raw_page_background" => self.raw_page_background.as_ref(),
            _ => None,
        }?;
        self.url_file(file)
    }

    fn url_file(&self, file: &str) -> Option<String> {
        let fmt = self.asset_url_format.as_ref()?;
        Some(format!(
            "https://shared.cloudflare.steamstatic.com/store_item_assets/{}",
            fmt.replace("${FILENAME}", file)
        ))
    }
}

#[derive(Default)]
struct Sources {
    cover: Option<DynamicImage>,
    hero: Option<DynamicImage>,
    logo: Option<DynamicImage>,
    icon: Option<DynamicImage>,
}

impl Sources {
    fn get(&self, kind: ArtKind) -> Option<&DynamicImage> {
        match kind {
            ArtKind::Cover => self.cover.as_ref(),
            ArtKind::Hero => self.hero.as_ref(),
            ArtKind::Logo => self.logo.as_ref(),
            ArtKind::Icon => self.icon.as_ref(),
        }
    }
}

/// Store search chokes on long, dash-separated names, so try shorter variants too:
/// `The Elder Scrolls III - Morrowind - GOTY` → `The Elder Scrolls III Morrowind`.
fn search_queries(title: &str) -> Vec<String> {
    let mut out = vec![title.to_string()];
    let parts: Vec<&str> = title.split(" - ").collect();
    if parts.len() >= 2 {
        out.push(format!("{} {}", parts[0], parts[1]));
        out.push(parts[1..].join(" "));
    }
    out.dedup();
    out
}

/// How well a store result name matches our title (0..=1), favouring editions of the
/// same game over DLC, soundtracks and spin-offs.
fn match_score(title: &str, name: &str, strict: bool) -> f32 {
    use crate::library::titles::normalize;
    let (nt, ni) = (normalize(title), normalize(name));
    let mut s = similarity(title, name);
    let lname = name.to_lowercase();
    let junk = ["soundtrack", "dlc", "season pass", "artbook", "demo", "playtest", "dedicated server",
        "clan of", "pack", "expansion", "bundle", "upgrade"];
    if junk.iter().any(|j| lname.contains(j)) {
        return s * 0.5;
    }
    if !strict && nt.len() >= 6 && ni.contains(&nt) {
        let extra = ni.replacen(&nt, "", 1);
        let editions = ["edition", "remaster", "definitive", "goty", "complete", "enhanced", "deluxe", "anniversary"];
        let edition_only = extra.is_empty() || editions.iter().any(|e| extra.contains(e));
        s = s.max(if edition_only { 0.95 } else { 0.82 });
    }
    s
}

pub(crate) fn titles_match(a: &str, b: &str) -> bool {
    let (na, nb) = (crate::library::titles::normalize(a), crate::library::titles::normalize(b));
    similarity(a, b) >= 0.7 || (na.len() >= 5 && nb.len() >= 5 && (na.contains(&nb) || nb.contains(&na)))
}

/// Heroes narrower than this are worth replacing with a sharper source when one exists.
const HQ_HERO_W: u32 = 2560;

fn load_file(p: &Path) -> Option<DynamicImage> {
    image::open(p).ok()
}

fn save(dir: &Path, kind: ArtKind, img: &DynamicImage) -> image::ImageResult<()> {
    let (mw, mh) = kind.max_size();
    let img = if img.width() > mw || img.height() > mh {
        img.resize(mw, mh, image::imageops::FilterType::CatmullRom)
    } else {
        img.clone()
    };
    let fmt = kind.format();
    let ext = if fmt == ImageFormat::Png { "png" } else { "jpg" };
    let path = dir.join(format!("{}.{ext}", kind.stem()));
    match fmt {
        ImageFormat::Jpeg => {
            let file = std::fs::File::create(&path)?;
            let quality = if kind == ArtKind::Hero { 95 } else { 90 };
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
            enc.encode_image(&img.to_rgb8())
        }
        _ => img.to_rgba8().save_with_format(&path, ImageFormat::Png),
    }
}

/// Saves `img` as `<stem>.jpg` under `dir`, downscaled to fit (`mw` × `mh`): the JPEG
/// half of `save`, for art files that aren't an `ArtKind` (hero alternates).
fn save_jpg(dir: &Path, stem: &str, img: &DynamicImage, mw: u32, mh: u32, quality: u8) -> image::ImageResult<()> {
    let img = if img.width() > mw || img.height() > mh {
        img.resize(mw, mh, image::imageops::FilterType::CatmullRom)
    } else {
        img.clone()
    };
    let file = std::fs::File::create(dir.join(format!("{stem}.jpg")))?;
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
    enc.encode_image(&img.to_rgb8())
}

/// Name variants to try against libretro's No-Intro / Redump based file names.
pub fn libretro_candidates(raw: &str) -> Vec<String> {
    let mut out = vec![raw.to_string()];
    // Drop language lists like "(En,Fr,De)" and revision tags.
    let mut kept = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('(') {
        let Some(end) = rest[start..].find(')') else { break };
        let group = &rest[start + 1..start + end];
        kept.push_str(&rest[..start]);
        let is_lang = group.contains(',') && !group.contains(' ');
        let is_rev = group.starts_with("Rev ") || group.starts_with("v1") || group.starts_with("Disc");
        if !is_lang && !is_rev {
            kept.push_str(&rest[start..start + end + 1]);
        }
        rest = &rest[start + end + 1..];
    }
    kept.push_str(rest);
    let kept = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if kept != raw {
        out.push(kept);
    }
    let base = strip_groups(raw);
    for region in ["(USA)", "(USA, Europe)", "(World)", "(Europe)", "(USA) (XBLA)", "(World) (XBLA)"] {
        out.push(format!("{base} {region}"));
    }
    out.push(base);
    let pretty = clean_title(raw);
    out.push(pretty);
    out.dedup();
    out
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libretro_variants() {
        let c = libretro_candidates("Sid Meier's Pirates! (USA, Europe) (En,Fr,De,Es,It)");
        assert_eq!(c[0], "Sid Meier's Pirates! (USA, Europe) (En,Fr,De,Es,It)");
        assert_eq!(c[1], "Sid Meier's Pirates! (USA, Europe)");
        let c = libretro_candidates("Star Wars - Knights of the Old Republic (USA) (Rev 1)");
        assert_eq!(c[1], "Star Wars - Knights of the Old Republic (USA)");
    }

    #[test]
    fn prefers_base_game_editions() {
        let base = match_score("Northgard", "Northgard: Definitive Edition", false);
        assert!(base > match_score("Northgard", "Northgard Battlegrounds", false));
        assert!(base > match_score("Northgard", "Northgard - Vordr, Clan of the Owl", false));
        assert!(match_score(
            "The Elder Scrolls III - Morrowind - Game of the Year Edition",
            "The Elder Scrolls III: Morrowind® Game of the Year Edition",
            true
        ) > 0.99);
        assert_eq!(search_queries("A - B - C"), vec!["A - B - C", "A B", "B C"]);
    }

    #[test]
    fn encodes() {
        assert_eq!(url_encode("A B's (USA)"), "A%20B%27s%20%28USA%29");
    }

    #[test]
    fn hero_variant_stems() {
        assert_eq!(hero_stem(0), "hero");
        assert_eq!(hero_stem(1), "hero2");
        assert_eq!(hero_stem(2), "hero3");
    }
}