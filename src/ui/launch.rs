//! Starting games and noticing when they finish.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};

use crate::library::{Game, Launch};

enum Watch {
    /// A child process; `dir` covers launchers that start the real game and exit.
    Child { child: Child, dir: Option<PathBuf> },
    /// Any process running from inside `dir`.
    Dir { dir: PathBuf },
    /// Steam reports the running app id in the registry.
    Steam { appid: u32, seen: bool },
}

pub struct RunningGame {
    pub title: String,
    watch: Watch,
    started: Instant,
    last_check: Instant,
    misses: u32,
    pub minimized: bool,
}

pub fn start(game: &Game) -> Result<RunningGame, String> {
    let watch = match &game.launch {
        Launch::Steam { appid, .. } => {
            if !crate::win::shell_open(&format!("steam://rungameid/{appid}")) {
                return Err("Steam didn't respond".into());
            }
            Watch::Steam { appid: *appid, seen: false }
        }
        Launch::Exe { exe, args, cwd, watch_dir } => {
            if !exe.is_file() {
                return Err(format!("{} not found", exe.display()));
            }
            let child = Command::new(exe)
                .args(args)
                .current_dir(if cwd.is_dir() { cwd.clone() } else { exe.parent().unwrap().to_path_buf() })
                .spawn()
                .map_err(|e| format!("couldn't start {}: {e}", exe.display()))?;
            Watch::Child { child, dir: watch_dir.clone() }
        }
        // Wishlist entries open their source page through `Action::OpenWishlist`; they never
        // come through here, but the match has to stay exhaustive.
        Launch::Url { .. } => return Err("this game is not installed".into()),
    };
    Ok(RunningGame {
        title: game.title.clone(),
        watch,
        started: Instant::now(),
        last_check: Instant::now(),
        misses: 0,
        minimized: false,
    })
}

impl RunningGame {
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Returns false once the game has exited. Cheap enough to call every frame
    /// (it only does real work once a second).
    pub fn poll(&mut self, sys: &mut sysinfo::System) -> bool {
        if self.last_check.elapsed() < Duration::from_millis(1000) {
            return true;
        }
        self.last_check = Instant::now();
        let elapsed = self.started.elapsed();

        match &mut self.watch {
            Watch::Child { child, dir } => match child.try_wait() {
                Ok(None) => true,
                _ => {
                    // Exited quickly? It was probably a launcher; follow the folder instead.
                    if elapsed < Duration::from_secs(30) {
                        if let Some(d) = dir.take() {
                            self.watch = Watch::Dir { dir: d };
                            self.misses = 0;
                            return true;
                        }
                    }
                    false
                }
            },
            Watch::Dir { dir } => {
                let dir = dir.clone();
                if any_process_in(sys, &dir) {
                    self.misses = 0;
                    true
                } else {
                    // Give slow launchers time before deciding the game is gone.
                    self.misses += 1;
                    self.misses < if elapsed < Duration::from_secs(45) { 30 } else { 3 }
                }
            }
            Watch::Steam { appid, seen } => {
                let running = crate::win::steam_running_appid();
                if running == *appid {
                    *seen = true;
                    true
                } else if *seen {
                    false
                } else {
                    // Steam may update or show dialogs first; wait up to 3 minutes.
                    elapsed < Duration::from_secs(180)
                }
            }
        }
    }
}

fn any_process_in(sys: &mut sysinfo::System, dir: &Path) -> bool {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let prefix = dir.to_string_lossy().to_lowercase();
    sys.processes().values().any(|p| {
        p.exe()
            .map(|e| e.to_string_lossy().to_lowercase().starts_with(&prefix))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ping_game(exe: PathBuf, count: &str, watch_dir: Option<PathBuf>) -> Game {
        Game {
            id: "test".into(),
            title: "Test".into(),
            system: crate::library::System::Pc,
            launch: Launch::Exe {
                cwd: exe.parent().unwrap().to_path_buf(),
                exe,
                args: vec!["-n".into(), count.into(), "127.0.0.1".into()],
                watch_dir,
            },
            steam_appid: None,
            art_name: None,
            last_played: None,
            controller: None,
            accent: None,
        }
    }

    fn poll_now(r: &mut RunningGame, sys: &mut sysinfo::System) -> bool {
        r.last_check = Instant::now() - Duration::from_secs(5);
        r.poll(sys)
    }

    #[test]
    fn child_exit_is_detected() {
        let mut sys = sysinfo::System::new();
        let mut r = start(&ping_game(PathBuf::from(r"C:\Windows\System32\PING.EXE"), "3", None)).unwrap();
        assert!(poll_now(&mut r, &mut sys), "should be running right after start");
        std::thread::sleep(Duration::from_secs(4));
        assert!(!poll_now(&mut r, &mut sys), "should notice the exit");
    }

    #[test]
    fn launcher_stub_follows_game_folder() {
        // A "launcher" that exits at once while the "game" keeps running from the same folder.
        let dir = std::env::temp_dir().join(format!("gdash-launch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let game_exe = dir.join("game.exe");
        std::fs::copy(r"C:\Windows\System32\PING.EXE", &game_exe).unwrap();
        let mut game = Command::new(&game_exe).args(["-n", "4", "127.0.0.1"]).spawn().unwrap();

        let stub = ping_game(PathBuf::from(r"C:\Windows\System32\PING.EXE"), "1", Some(dir.clone()));
        let mut sys = sysinfo::System::new();
        let mut r = start(&stub).unwrap();
        // Give the stub time to exit; on a busy CI machine a fixed 600 ms can be too tight.
        let mut followed = false;
        for _ in 0..20 {
            if !poll_now(&mut r, &mut sys) {
                break;
            }
            if matches!(r.watch, Watch::Dir { .. }) {
                followed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(followed, "stub should hand over to the game folder");
        assert!(poll_now(&mut r, &mut sys), "game still running");

        game.wait().unwrap();
        r.started = Instant::now() - Duration::from_secs(120);
        let mut alive = true;
        for _ in 0..5 {
            alive = poll_now(&mut r, &mut sys);
        }
        assert!(!alive, "should give up once nothing runs from the folder");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
