# gdash

A fullscreen, controller-first game launcher with a console-style "showcase shelf" UI, written in
Rust with [Bevy](https://bevy.org). It finds your installed Steam games, native PC games and
emulated Xbox / Xbox 360 images, fetches artwork for them, and launches them from a controller
or keyboard and mouse.

The look is an original homage: own shader, colours and synthesised sounds, Windows' own
Segoe UI fonts. No Microsoft assets are included.

## Install

```
powershell -ExecutionPolicy Bypass -File install.ps1
```

Builds the release exe, copies it (with its icon built in) to `%LOCALAPPDATA%\Programs\gdash`
and adds a Start Menu shortcut, so typing "gdash" in Start finds it like any other app. Run it
again to update after changes (exit gdash first); `-NoBuild` reuses the last build and
`-Uninstall` removes it (your config and art cache stay). If "Start with Windows" is on, the
script points it at the installed copy.

gdash is a Windows GUI program (no console window). The command-line flags below still print
to the terminal they're run from.

## Running from source

```
cargo run --release
```

The first launch scans your drives for the usual layouts and writes
`%APPDATA%\gdash\config\config.toml`. The library is cached, so later launches show
the dashboard immediately and refresh it in the background.

| Flag | What it does |
|---|---|
| `--windowed` | Run in a window instead of borderless fullscreen |
| `--scan` | Print the discovered library as JSON (game ids for config overrides) and exit |
| `--fetch-art` | Download missing artwork and print a coverage table |
| `--refresh-art` | Retry artwork that previously wasn't found |
| `--enhance-art` | Run the ML pass now (depth maps, segmentation, upscaling); the dashboard also does this in the background |
| `--nav <keys> --screenshot <file.png>` | Scripted navigation (`L R` move, `Q E` category, `A B Y G` buttons, `K P` keyboard/controller prompts, `X` dry-run launch burst, `W` wait 4 s) then a screenshot; used for visual checks |

## Controls

| Action | Controller | Keyboard / mouse |
|---|---|---|
| Move along the shelf | D-pad / left stick | ← → / mouse wheel / hover |
| Move in the All grid | D-pad / left stick (↑ ↓ by row) | Arrow keys / hover |
| Switch category | LB / RB | Q / E, Tab / Shift+Tab, click a tab |
| Jump 5 (3 rows in the grid) | LT / RT | Page Up / Page Down |
| Play / open wishlist page | A | Enter / Space / click |
| Details | Y | I / Y |
| Back (closes overlays, else Recent) | B | Backspace / right click |
| Guide (rescan, refresh art, exit) | Start / View | Esc / Home |
| Look around the background | Right stick | Move the mouse |

Button prompts follow whatever you last touched: plugging in or picking up a controller flips
them to controller glyphs, typing or deliberately moving the mouse flips them back.

While a game runs the dashboard minimises and drops to about one update per second. It comes
back when the game exits. Input is ignored while the dashboard isn't focused, so pressing A in
a game never launches anything.

The look is specified in [design/SPEC.md](design/SPEC.md), with interactive HTML mockups beside
it (serve `design/` over http to view them with real art).

The **All** tab (first tab) shows every game at once as a grid of box art, sized to fit the
window (12 columns × 4 rows on a 16:9 screen), with the selected game's name above it and its
hero image behind. The dashboard still opens on Recent.

The **Wishlist** tab shows games from a plain URL list of things you might download later, in
the same grid. It reads `00_TO_DOWNLOAD.txt` from each `pc_roots` folder (so `G:\Games\00_TO_DOWNLOAD.txt`
works automatically), or a file set with `wishlist_file`. One page URL per line; blank lines and
`#` / `//` comments are ignored. A line can also be `Title | URL` to choose the display title.
Titles come from the URL slug otherwise. Entries already installed are hidden on rescan, and
pressing **A** opens the source page in your browser instead of launching anything. Artwork is
looked up from Steam by title, with the normal title-card fallback when no match is found.

The **Controller** tab shows only the games you can actually play with a gamepad — full
support first, then partial, alphabetical within each. Games with no controller support, and
ones that couldn't be checked, stay off the tab entirely. Where the verdicts come from, most
reliable first:

1. your config: `controller = "full" | "partial" | "none"` under `[overrides.<id>]` always wins
2. the Steam store page of the game's app id (its `controller_support` tag)
3. for PC games without one: a Steam store search by title, then that page's tag
4. the game's own files: XInput / DirectInput / SDL game-controller DLLs next to the exe mean a
   gamepad works at least in game (partial)

Emulated Xbox games are console games, so they always show full support. Lookups run on the
background scan and are cached in `data\controllers.json`; **Refresh artwork** (guide menu)
retries ones that came back unknown, and a config override fixes any game for good.

## Where games come from

- **Steam**: `libraryfolders.vdf` and `appmanifest_*.acf` (tools and redistributables are
  skipped). Launched with `steam://rungameid/<id>`. Last-played times come from Steam.
- **PC**: every sub-folder of `pc_roots` is one game. The main exe is picked by a score (name
  match with the folder, depth, size, with installers, crash handlers and launchers ranked
  down). Override it per game in the config.
- **Xbox** (xemu) and **Xbox 360** (Xenia): `.iso` / `.xiso.iso`, `default.xex`, and
  XBLA/GoD packages. Xenia's `recent.toml` supplies titles and play times.
- **Wishlist**: page URLs from `00_TO_DOWNLOAD.txt` (or `wishlist_file`). These are not
  installed games; they are only browsable and open their page in the browser.

## Look and effects

The background is drawn in the world on an HDR camera, so Bevy's post stack (bloom, chromatic
aberration, lens distortion) applies to it while the UI stays sharp. Each game gets:

- its **own colour**, picked from its box art, used for the cover glow, Play glow and lighting
- a **living hero**: the full-width image is cut into rigid cards from the depth map. Every
  connected subject (and anyone segmentation marks as a person or animal) is one flat card at
  a single depth, so it moves as a piece and never bends; the seams are pushed out into the
  background, where a little stretch of cloud or scenery goes unnoticed. Each game
  gets one motion style: *breathe* (near layers swell toward you), *glide* (a slow sideways
  parallax) or *push-in* (a very slow zoom into the subject), centred on the depth model's
  focal point, with light swelling on near subjects
- **effects that come out of the picture**: each hero is read for fire (hot, bright, glowing
  colours), small lights, a dark starry sky, water and tree canopies. Fire flickers and blooms
  and sheds embers that rise off it; lights and stars twinkle and now and then flash a glint;
  water gets a slow sheen and the odd glint; leaves drift down from the trees; dust hangs around
  lights. Particles are pinned to the depth card they came from, so they move with the image
- its **own atmosphere**: light beams from the top corners, and, only for art with nothing of
  its own to animate, a light touch of dust, snow/ash, bokeh or sparks (or nothing), chosen from
  the game's id and colour

### ML artwork enhancement

After fetching art, the dashboard runs `gdash --enhance-art` as a hidden, low-priority child
process (so the models' memory never stays in the dashboard) and cross-fades improved art in as
it finishes. Each image is processed once, on the CPU:

- **Depth** — [Depth Anything V2 Small](https://huggingface.co/onnx-community/depth-anything-v2-small)
  (Apache-2.0, int8, 27 MB) writes `depth.png` for every hero; it picks the focal point and drives the layered motion and relighting
  (about 0.6 s per image).
- **Segmentation** — [SegFormer-B0 (ADE20K)](https://huggingface.co/Xenova/segformer-b0-finetuned-ade-512-512)
  (NVIDIA SegFormer licence, ONNX export by Xenova, 15 MB) writes `segment.png` (sky, water,
  foliage, people) for every hero in overlapping square crops (about 0.4 s per image). It
  places the stars, water glints and leaves, and keeps people rigid.
- **Upscaling** — [Real-ESRGAN General x4v3](https://huggingface.co/qualcomm/Real-ESRGAN-General-x4v3)
  (BSD-3-Clause, ONNX export by Qualcomm AI Hub, 4.5 MB) upscales heroes under 1000 px tall
  in 128 px tiles (about 8 s for a 1920-wide hero). For 2x results a little of a plain bicubic
  upscale is mixed back in so painted texture isn't smoothed away. The original is kept as
  `hero-original.*`.

Models download on first use to `%LOCALAPPDATA%\gdash\data\models` and are checked against
pinned SHA-256 hashes. ONNX Runtime is linked into the exe by the `ort` crate. Turn it off with
`enhance_art = false` in the config. Without the depth model, a quick built-in estimate is used;
your own grayscale `depth.png` (white = near) always wins.

Launching plays a burst: a flash in the game's colour that blooms, a lens bulge, and the
cover lifting off the shelf.

## Artwork

Cached under `%LOCALAPPDATA%\gdash\data\art\<game-id>\` as `cover`, `hero`, `logo` and `icon`.
Sources, in order:

1. Your own file in that folder (always wins)
2. Steam's local library cache
3. Steam store CDN, by app id or store-name search (keyless)
4. libretro-thumbnails box art and screenshots for Xbox / Xbox 360
5. The game exe's own icon
6. A generated title card when nothing else exists

## Config example

```toml
pc_roots = ['G:\Games']
wishlist_file = 'G:\Games\00_TO_DOWNLOAD.txt'  # optional; auto-detected in pc_roots
hidden = ["pc-some-tool"]

[overrides.pc-mass-effect]
exe = 'MassEffectLauncher.exe'   # relative to the game folder, or absolute
args = []
title = "Mass Effect"
steam_appid = 17460              # used for artwork lookup
controller = "full"             # skip the lookup: full / partial / none
```

## Layout

```
src/
  main.rs          CLI flags
  config.rs        config.toml + first-run detection
  library/         scanners: steam, native (exe scoring), emu (xemu/xenia), wishlist (to-download
                   URL list), controller (gamepad support), vdf parser, titles
  art.rs           artwork fetch + cache
  win.rs           registry, ShellExecute, exe icons, focus, clock
build.rs           embeds the app icon (assets/gdash.ico, drawn by assets/make-icon.ps1)
install.ps1        per-user install + Start Menu shortcut
  enhance.rs       ML pass: depth maps, segmentation and super-resolution (ONNX Runtime)
  ui/              Bevy app: shelf view, All grid (grid.rs), input, textures, depth cards (depth.rs), scene
                   analysis (scene.rs), image particles (particles.rs), background shader,
                   post effects (fx.rs), sounds, launch
```

`cargo test` covers the VDF parser, wishlist URL/title parsing, exe picking on real folder layouts, title clean-up, artwork
name matching, the launch/exit watcher, colour picking, depth estimation and rigid depth cards, scene analysis (fire vs. skin/roof
tiles), the particle screen mapping and per-game looks. `cargo test dump_scene -- --ignored`
(with `GDASH_DEPTH_OUT` and `GDASH_DEPTH_IDS`) draws what the analysis found over each hero.
