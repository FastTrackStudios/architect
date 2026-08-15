//! Headless wry capture: spawn the consuming app on Xvfb, wait for
//! the window to map, screenshot it via ImageMagick, kill everything.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum WryCaptureError {
    #[error("failed to spawn `{tool}`: {source}")]
    SpawnFailed {
        tool: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("`{tool}` exited with status {status:?}: {stderr}")]
    ToolFailed {
        tool: &'static str,
        status: Option<i32>,
        stderr: String,
    },
    #[error("timed out waiting for snapshot window `{title}` after {millis}ms")]
    WindowTimeout { title: String, millis: u64 },
    #[error("failed to read screenshot file: {0}")]
    ReadFailed(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct WryCaptureConfig {
    /// Path to the consumer's compiled desktop binary (e.g.
    /// `target/release/architect-ui-showcase-desktop`). Must accept a
    /// `--snapshot=<story>` argument.
    pub binary: PathBuf,
    /// Xvfb display number. Picked to avoid clashing with any local
    /// X server (`:0`/`:1`). Use a value ≥ `:99` in CI.
    pub display: u32,
    /// Virtual screen resolution. Should match (or exceed) the
    /// inner-window size declared by the binary in `--snapshot` mode.
    pub screen_w: u32,
    pub screen_h: u32,
    /// How long to wait for the window to appear before failing.
    pub window_timeout: Duration,
    /// Extra paint settling delay AFTER the window is mapped. wry
    /// reports the window as mapped before the first paint completes,
    /// so a small grace period stops us screenshotting a blank
    /// surface. 700ms is conservative; lower if your stories are
    /// fully synchronous.
    pub paint_grace: Duration,
}

impl Default for WryCaptureConfig {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("target/release/architect-ui-showcase-desktop"),
            display: 99,
            screen_w: 1280,
            screen_h: 800,
            window_timeout: Duration::from_secs(15),
            paint_grace: Duration::from_millis(2000),
        }
    }
}

/// Spawn the binary against an internally-managed Xvfb instance,
/// screenshot the snapshot window, return PNG bytes.
pub fn capture_wry_via_xvfb(
    cfg: &WryCaptureConfig,
    story_name: &str,
) -> Result<Vec<u8>, WryCaptureError> {
    let display = format!(":{}", cfg.display);
    let mut xvfb = spawn_xvfb(cfg)?;

    // Give Xvfb a beat to bind the socket. Cheap.
    sleep(Duration::from_millis(150));

    let result = capture_inner(cfg, &display, story_name);

    // Always tear down Xvfb, regardless of result.
    let _ = xvfb.kill();
    let _ = xvfb.wait();

    result
}

fn capture_inner(
    cfg: &WryCaptureConfig,
    display: &str,
    story_name: &str,
) -> Result<Vec<u8>, WryCaptureError> {
    let title = format!("fts-ui-snapshot:{story_name}");

    let mut app = Command::new(&cfg.binary)
        .arg(format!("--snapshot={story_name}"))
        .env("DISPLAY", display)
        // Force software GL + disable DMA-BUF + force X11 backend so
        // webkitgtk doesn't try (and hang) initialising hardware
        // acceleration against an Xvfb display that has no GPU. These
        // are mandatory on bare Xvfb — the binary appears to launch
        // successfully without them but the WebView never paints.
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
        .env("WEBKIT_DISABLE_COMPOSITING_MODE", "1")
        .env("GDK_BACKEND", "x11")
        // Inherit so the binary's panic traces / GTK warnings reach
        // the test output instead of being silently dropped.
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| WryCaptureError::SpawnFailed {
            tool: "consumer binary",
            source: e,
        })?;

    let result = (|| {
        let window_id = wait_for_window(display, &title, cfg.window_timeout)?;
        sleep(cfg.paint_grace);
        screenshot_window(display, &window_id)
    })();

    // Surface early-exit info before we kill — if the wry binary died
    // on its own (e.g. webkitgtk missing) the next try_wait tells us so.
    if let Ok(Some(status)) = app.try_wait() {
        eprintln!(
            "architect-story-parity: binary exited on its own with status {status:?} (likely webkitgtk \
             / GTK init failure on the Xvfb display)"
        );
    }

    let _ = app.kill();
    let _ = app.wait();

    result
}

fn spawn_xvfb(cfg: &WryCaptureConfig) -> Result<Child, WryCaptureError> {
    // Pin Xvfb DPI to 96 so 1 CSS pixel == 1 device pixel in webkit.
    // Default Xvfb DPI is 75, which makes webkit scale every CSS-pixel
    // value by 96/75 ≈ 1.28 (or some implementation-specific factor) —
    // a `width: 32px` div renders as 33–34 device pixels and text
    // baselines drift ~1 px per row. Blitz's CPU rasteriser uses
    // CSS-pixel == device-pixel at scale=1.0, so the only way to get
    // pixel-perfect parity is to make Xvfb agree.
    Command::new("Xvfb")
        .arg(format!(":{}", cfg.display))
        .arg("-screen")
        .arg("0")
        .arg(format!("{}x{}x24", cfg.screen_w, cfg.screen_h))
        .arg("-dpi")
        .arg("96")
        .arg("-nolisten")
        .arg("tcp")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| WryCaptureError::SpawnFailed {
            tool: "Xvfb",
            source: e,
        })
}

fn wait_for_window(
    display: &str,
    title: &str,
    timeout: Duration,
) -> Result<String, WryCaptureError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let out = Command::new("xdotool")
            .arg("search")
            .arg("--name")
            .arg(title)
            .env("DISPLAY", display)
            .output();
        match out {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !s.is_empty() {
                    // Take the first match (most recent window).
                    let id = s.lines().next().unwrap_or("").to_string();
                    if !id.is_empty() {
                        return Ok(id);
                    }
                }
            }
            Ok(_) => {} // exit 1 = no window yet, keep polling
            Err(e) => {
                return Err(WryCaptureError::SpawnFailed {
                    tool: "xdotool",
                    source: e,
                });
            }
        }
        sleep(Duration::from_millis(120));
    }
    Err(WryCaptureError::WindowTimeout {
        title: title.to_string(),
        millis: timeout.as_millis() as u64,
    })
}

fn screenshot_window(display: &str, window_id: &str) -> Result<Vec<u8>, WryCaptureError> {
    // ImageMagick `import` writes to a path; pipe via /tmp.
    let tmp =
        std::env::temp_dir().join(format!("architect-story-parity-{}.png", std::process::id()));
    let out = Command::new("import")
        .arg("-window")
        .arg(window_id)
        .arg(&tmp)
        .env("DISPLAY", display)
        .output()
        .map_err(|e| WryCaptureError::SpawnFailed {
            tool: "import (imagemagick)",
            source: e,
        })?;
    if !out.status.success() {
        return Err(WryCaptureError::ToolFailed {
            tool: "import",
            status: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        });
    }
    let bytes = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    Ok(bytes)
}

/// Sanity check — call once at the start of a parity run to fail fast
/// with a clear message if the host system lacks the required tools.
///
/// We can't reliably probe `--version` because Xvfb exits non-zero on
/// `-version`. Instead we resolve each command on `$PATH` via `which`
/// and treat presence as success.
pub fn check_dependencies() -> Result<(), String> {
    fn has(cmd: &str) -> bool {
        // Spawn `which <cmd>` rather than running the binary itself —
        // some tools (Xvfb, ImageMagick `import`) reject every probe
        // flag we'd try and exit non-zero. `which` from coreutils is
        // present on every POSIX system we care about.
        Command::new("which")
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    let mut missing = Vec::new();
    if !has("Xvfb") {
        missing.push("Xvfb");
    }
    if !has("xdotool") {
        missing.push("xdotool");
    }
    if !has("import") {
        missing.push("import (imagemagick)");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "architect-story-parity needs: {} — install via your package manager",
            missing.join(", ")
        ))
    }
}

/// Helper for tests / CLIs — write the captured PNG to disk.
pub fn write_png(bytes: &[u8], path: impl AsRef<Path>) -> std::io::Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}
