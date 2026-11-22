# gdash UI spec: Showcase Shelf

The reference mockup is `b-showcase-shelf.html`. Serve `design/` over http (the `design-mockups`
entry in `.claude/launch.json`) to see it with real art.

## Principles

- **The game art is the interface.** The hero image fills the screen. UI sits on gradients
  that darken only where text needs it, never on an opaque panel.
- **Colour is an accent, not a wash.** The category colour appears in the active tab
  underline and stands by as the fallback. The selected tile's glow and the Play
  button's glow carry the selected game's own signature colour, lifted from its art
  (the most prominent saturated hue; grey art falls back to the category's). Pin a
  game's colour by hand with `accent = "#rrggbb"` under its `[overrides.<id>]` in
  config.toml.
- **One focus at a time.** Exactly one tile is lifted, ringed and glowing. Everything else
  steps back (72% opacity, smaller).
- **Everything moves, gently.** Selection, category changes and hero swaps all ease
  (cubic ease-out, 250–800 ms). Nothing pops.

## Canvas

Layouts are authored on a 900-px-tall canvas; `UiScale = window height / 900`. A 16:9 screen
is 1600 × 900 canvas px. The side margin is 72.

## Layers (back to front)

1. **Background shader**
   - The hero image is cover-fit, biased toward the top, and cross-fades over 0.8 s.
   - A slow 1.00 → 1.04 zoom runs over 12 s after each change.
   - Shade gradients, multiplied toward `#030508` (smooth curves, no hard edges):
     - left: 92% at x = 0, 55% at 32%, 0 at 62%
     - bottom: 30% at the bottom edge, easing smoothly to 0 at 55%
     - top: 70% at the top edge, easing smoothly to 0 at 18%
   - Without a hero, the background is dark ambient light ribbons tinted with the accent.
2. **Top bar** (top 34)
   - The orb, then an `LB` key cap.
   - Category tabs, 22 px with a 34 px gap. Inactive tabs are 50% white; the active tab is
     semibold with a 3 px accent underline and glow.
   - An `RB` key cap, then status and the clock on the right.
3. **Info block** (left 72, sits 40 px above the shelf)
   - The logo, max 560 × 190, with a drop shadow. Without a logo: the title at 58 px
     semibold.
   - A meta line at 18 px, 80%: system • last played.
   - Actions: a white `A Play` pill with an accent glow, and a glass `Y Details` pill.
     Both are 52 px tall.
4. **Shelf** (bottom 70)
   - Box-art tiles are 170 × 255; the selected tile is 210 × 315. Gap 22.
   - The selected tile has a 3 px white ring, an accent glow and a deep shadow.
   - The selection stays the third tile from the left, and the shelf scrolls.
   - The Recent category shows a system badge on each tile.
   - Without cover art: a gradient card with the title.
   - System entries: square glass cards with a Fluent icon.
5. **Count** `3 / 12`, right-aligned above the shelf.
6. **Prompts** (bottom 24, 15 px, 70%): A Play · B Back · Y Details · ≡ Guide.
7. **Overlays**
   - Details sheet (right side, glass). The game's art as a living card at the top —
     cover-fit, drifting slowly around the scene's focal point, revealing as the glass
     racks in — then the title, the meta line and typographic rows: SYSTEM, CONTROLLER
     (with a three-notch meter), LAST PLAYED, LOCATION. Wishlist items show STATUS,
     SOURCE and LINK instead. The config id sits under the rows, dim.
   - Guide (center, glass).
   - Toast.
   - Now playing.
   - Launch splash (full-screen fade to black with the game's logo stamped in it).

## Category colours

| Recent | Steam | PC | Xbox | Xbox 360 | Controller | Wishlist | System |
|---|---|---|---|---|---|---|---|
| `#2fd0c0` | `#4d9bff` | `#ff9a3c` | `#6fdc3a` | `#d65bd0` | `#8f7bff` | `#ff82a3` | `#a7b4c4` |

**Wishlist** uses the All grid layout. Its items are not installed; `A` opens the source page
in the browser, and the info line shows `Not installed · <source host>`.

**Controller** uses the All grid layout too, but shows only games that actually work with a
gamepad: full support first, then partial, alphabetical within each. The grid's info line
leads with the verdict. Games with no support or unknown ones stay off the tab.

## Controls

| | Controller | Keyboard / mouse |
|---|---|---|
| Move along the shelf | D-pad / stick left-right | ← → / wheel / hover |
| Category | LB / RB | Q / E, Tab / Shift+Tab, click a tab |
| Jump 5 | LT / RT | Page Up / Down |
| Play | A | Enter / Space / click |
| Details | Y | I / Y |
| Back | B (closes overlays, else first category) | Backspace / Esc in overlays |
| Guide | Start / View | Esc / Home |

## Motion

- **Tiles:** size and position use an exponential ease (rate 14/s).
- **Press:** A on a game dips the selected cover and the Play pill for ~120 ms, spiking its
  glow; on a real launch it flows straight into the launch sequence.
- **Launch:** the cover dips and flares (0–0.15 s), the hero swells from its showcase crop
  to full-bleed while pushing in (0–0.9 s), the screen sinks to black and the game's logo —
  or its name, set big — stamps into the dark (0.5–1.6 s), and the game takes over.
- **Category change:** the new shelf rises 60 px and fades in, staggered 40 ms per tile. The
  info block slides 16 px from the left and fades in over 250 ms. The new hero enters from
  the side you moved and settles; the old one slides out of its way.
- **Grid ↔ shelf:** when the move also changes layouts (All / Wishlist / Controller ↔ a
  shelf), the game you land on keeps its tile: the cover detaches from the cell it held in
  the grid — or its slot on the shelf — and flies to where it lands, half a second, cubic
  ease-out with a slight bow and lean. The rest of the new view racks in around the
  reserved slot, whose own tile waits hidden until the cover arrives and swaps in, so it
  reads as one tile moving through one space, not two layouts cutting.
- **Hero variety:** a game can carry alternate heroes (`hero2` / `hero3` in its art folder —
  fetched from the store's screenshots, or dropped in by hand). While a game stays selected
  and nothing is open, its hero quietly cross-fades to the next alternate every ~13 s — no
  direction, the same motion style and atmosphere, the usual front-to-back dissolve. Overlays
  pause it; a launch owns the screen.
- **Idle ambient:** left untouched for two minutes, the interface racks itself away — the
  top bar lifts out, the info block and prompts slide off, the shelf (or the grid with its
  header) sinks — and the hero spreads to a full-bleed backdrop with its UI shading eased
  off and its drift slowed; hero alternates keep rotating. Any touch racks it all back,
  and the first input only restores — it never presses what it landed on. Overlays and a
  launch keep the interface. The art never stops moving (parallax, motion styles,
  alternates), so long ambient stays OLED-safe.
- **Startup:** fade in from black over 0.6 s, with the same shelf rise.

## Type

Segoe UI Variable with optical sizing when it's installed (Windows 11), classic Segoe UI
otherwise. The clock, the status line and the `n / total` counters use tabular figures so
they hold still as digits change. The 58 px title tightens its tracking; small uppercase
labels (the Recent badge) loosen theirs, as in the mockup.

## Grain

The background carries a screen-anchored whisper of grain (triangular dither, ±0.0012
linear). It kills banding in the smooth dark gradients on 8-bit panels and keeps the big
soft washes alive; the art drifts under it, the grain doesn't.
