# gdash — project instructions

A fullscreen, controller-first game launcher for Windows: Rust + Bevy (pinned `=0.20.1`),
ML-enhanced hero art, console-style Showcase Shelf UI. The design language lives in
`design/SPEC.md` and the HTML mockups in `design/`; treat them as the source of truth for
visuals and keep drift between them and the code deliberate.

## Ship after finishing work

- When a coherent piece of work is done and ready to test on the real machine, build and
  install it **without being asked**: run
  `powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1`
  (release build, copies to `%LOCALAPPDATA%\Programs\gdash`, refreshes the Start Menu
  shortcut). Run it in the background and say so; report the result when it completes.
- Don't install mid-iteration — only once the change set is complete and `cargo test`
  passes. A one-line doc tweak doesn't need an install.
- `install.ps1` fails if gdash is running. Tell the user to exit it first
  (guide menu > Exit gdash) and offer to retry; never kill their running app.

## Verify before shipping

- `cargo test` must pass.
- Visual changes: verify with the app's scripted screenshot mode, e.g.
  `cargo run -- --windowed --nav "RWW" --screenshot <file.png>`
  Script letters: `U D L R` move, `Q E` category, `A B Y G` buttons, `K P` device
  prompts, `X` dry-run launch burst, `W` wait 4 s. Dev profile is fine for screenshots;
  only `install.ps1` builds release.

## Conventions

- Styling is code, not CSS: colors and layout constants in `src/ui/theme.rs`.
- The background and particles render in the world on an HDR camera so Bevy's post
  stack (bloom, CA, lens) hits them; the UI stays sharp on top. New screen effects
  belong in `src/ui/shaders/` (WGSL for Material2d, WESL for UI materials).
- Overlays sample the glass backdrop via the offscreen copy camera (`src/ui/glass.rs`);
  anything new that must show the blurred picture behind it goes through that.
