//! `cargo xtask dist`: build a distribution of Architect (#491) — the
//! desktop app, the web bundle and image, the node binary and image —
//! from a distribution crate, this repo's `architect-desktop` by default.
//!
//! A distribution crate is a binary whose `main` is one line,
//! `architect_desktop::run(<its Distribution>)`, with a `gui` feature on
//! by default that turns on `architect-desktop/gui`
//! (`docs/content/guide/your-own-architect.md`). Everything is built
//! the way the release builds the Architect app, by the same scripts:
//!
//! | target | how | out |
//! | --- | --- | --- |
//! | `desktop` | the window's build, then `scripts/desktop/linux.sh` (an `AppImage`) or `scripts/desktop/macos.sh` (a dmg) | `<Name>-linux-x86_64.AppImage` |
//! | `node` | `--no-default-features` and `--cfg no_workspace_hack`, as the release's node | the binary |
//! | `node-image` | the node in `flake.nix`'s `architect-image-prebuilt` | `<folder>-node.tar` (docker-archive) |
//! | `web` | `scripts/web-bundle.sh`, with the node's `distribution` as `/app/distribution.json` | `web/` |
//! | `web-image` | the bundle in `flake.nix`'s `architect-web-image-prebuilt` | `<folder>-web.tar` |
//!
//! Who the app is (its name, binary, bundle id, folder, icon) is asked
//! of the built binary (`<bin> distribution`, `<bin> distribution icon`),
//! never written twice. Each artifact is printed last as `key=path`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What to build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Target {
    Desktop,
    Node,
    NodeImage,
    Web,
    WebImage,
    All,
}

/// How `cargo xtask dist` was asked.
#[derive(Debug, Clone)]
pub struct Options {
    pub targets: Vec<Target>,
    /// The distribution crate's `Cargo.toml`; none is this repo's
    /// `architect-desktop`.
    pub manifest_path: Option<PathBuf>,
    /// Its binary; none is the crate's only one (`architect-desktop`
    /// here).
    pub bin: Option<String>,
    pub out: PathBuf,
    pub profile: String,
    pub target_dir: Option<PathBuf>,
    /// The version written into the desktop entry and Info.plist.
    pub version: Option<String>,
}

/// The repo: the scripts and the flake are read from here wherever the
/// command runs.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

/// The host's target triple (`rustc -vV`).
fn host() -> Result<String, String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|e| format!("rustc -vV: {e}"))?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: ").map(str::to_owned))
        .ok_or_else(|| "rustc -vV named no host".to_owned())
}

/// Builds the distribution's binary, the window's (`node == false`) or
/// the node's, and answers its path (cargo's own report of it).
fn build(opts: &Options, node: bool) -> Result<PathBuf, String> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(root())
        .args(["build", "--message-format", "json-render-diagnostics"])
        .args(["--profile", &opts.profile]);
    match &opts.manifest_path {
        Some(m) => {
            cmd.arg("--manifest-path").arg(m);
        }
        None => {
            cmd.args(["-p", "architect-desktop"]);
        }
    }
    if let Some(bin) = &opts.bin {
        cmd.args(["--bin", bin]);
    } else if opts.manifest_path.is_none() {
        cmd.args(["--bin", "architect-desktop"]);
    } else {
        cmd.arg("--bins");
    }
    if let Some(dir) = &opts.target_dir {
        cmd.arg("--target-dir").arg(dir);
    }
    if node {
        // Without the window (no Blitz, wgpu or wayland), and without the
        // workspace-hack, which would pull the GUI stack's features back
        // in: the release's node build.
        cmd.arg("--no-default-features");
        if opts.manifest_path.is_none() {
            // A distribution crate depends on architect-desktop with
            // `vox` (its node's services) itself.
            cmd.args(["--features", "vox"]);
        }
        cmd.arg("--config").arg(format!(
            "target.{}.rustflags=[\"--cfg\",\"no_workspace_hack\"]",
            host()?
        ));
    }
    eprintln!("dist: {cmd:?}");
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cargo build: {e}"))?;
    if !out.status.success() {
        return Err("cargo build failed".to_owned());
    }
    let mut found = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if msg.get("reason").and_then(|r| r.as_str()) == Some("compiler-artifact")
            && let Some(exe) = msg.get("executable").and_then(|e| e.as_str())
        {
            found = Some(PathBuf::from(exe));
        }
    }
    found.ok_or_else(|| "cargo built no binary".to_owned())
}

/// What the binary says it is (`<bin> distribution`).
#[derive(Debug, Clone)]
struct Described {
    json: String,
    name: String,
    folder: String,
    bundle_id: String,
}

fn describe(bin: &Path) -> Result<Described, String> {
    let ask = |args: &[&str]| -> Result<String, String> {
        let out = Command::new(bin)
            .args(args)
            .output()
            .map_err(|e| format!("{} {}: {e}", bin.display(), args.join(" ")))?;
        if !out.status.success() {
            return Err(format!(
                "{} {} failed: {}",
                bin.display(),
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let json = ask(&["distribution"])?;
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| {
        format!(
            "{} distribution is not JSON ({e}): is its main `architect_desktop::run`?",
            bin.display()
        )
    })?;
    let field = |k: &str| {
        v.get(k)
            .and_then(|f| f.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    Ok(Described {
        name: field("name"),
        folder: field("folder"),
        bundle_id: field("bundle_id"),
        json,
    })
}

fn icon(bin: &Path, out: &Path) -> Result<PathBuf, String> {
    let svg = Command::new(bin)
        .args(["distribution", "icon"])
        .output()
        .map_err(|e| format!("{} distribution icon: {e}", bin.display()))?;
    let path = out.join("icon.svg");
    std::fs::write(&path, &svg.stdout).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

fn run_script(cmd: &mut Command) -> Result<(), String> {
    eprintln!("dist: {cmd:?}");
    let status = cmd
        .current_dir(root())
        .status()
        .map_err(|e| format!("{cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed ({status})"))
    }
}

/// An image's stream script from the flake (`nix build --impure`), run
/// into `tar` as a docker-archive.
fn image(
    attr: &str,
    env: &[(&str, &std::ffi::OsStr)],
    name: &str,
    tar: &Path,
) -> Result<(), String> {
    let mut nix = Command::new("nix");
    nix.args(["build", "--impure", "--no-link", "--print-out-paths"])
        .arg(format!("{}#{attr}", root().display()))
        .env("ARCHITECT_IMAGE_NAME", name);
    for (k, v) in env {
        nix.env(k, v);
    }
    eprintln!("dist: {nix:?}");
    let out = nix
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("nix build: {e}"))?;
    if !out.status.success() {
        return Err(format!("nix build .#{attr} failed"));
    }
    let streamer = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let file = std::fs::File::create(tar).map_err(|e| format!("{}: {e}", tar.display()))?;
    let status = Command::new(&streamer)
        .stdout(file)
        .status()
        .map_err(|e| format!("{streamer}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{streamer} failed"))
    }
}

/// The file name a title makes (`Architect Sandbox` → `ArchitectSandbox`).
fn file_title(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// `cargo xtask dist <targets…>`.
///
/// # Errors
///
/// A build, a script or an image failed; the message says which.
pub fn run_cli(opts: &Options) -> Result<(), String> {
    let all = opts.targets.contains(&Target::All);
    let wants = |t: Target| all || opts.targets.contains(&t);
    std::fs::create_dir_all(&opts.out).map_err(|e| format!("{}: {e}", opts.out.display()))?;
    let out = opts
        .out
        .canonicalize()
        .map_err(|e| format!("{}: {e}", opts.out.display()))?;
    let mut made: Vec<(&str, PathBuf)> = Vec::new();

    let node = if wants(Target::Node)
        || wants(Target::NodeImage)
        || wants(Target::Web)
        || wants(Target::WebImage)
    {
        Some(build(opts, true)?)
    } else {
        None
    };
    if let Some(bin) = &node {
        made.push(("node", bin.clone()));
    }

    if wants(Target::Desktop) {
        let bin = build(opts, false)?;
        let d = describe(&bin)?;
        let svg = icon(&bin, &out)?;
        let file = bin
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = file_title(&d.name);
        let mut script = Command::new("bash");
        let mac = cfg!(target_os = "macos");
        script
            .arg(root().join(if mac {
                "scripts/desktop/macos.sh"
            } else {
                "scripts/desktop/linux.sh"
            }))
            .env("SKIP_BUILD", "1")
            .env("APP_BIN_PATH", &bin)
            .env("APP_BIN", &file)
            .env("APP_TITLE", &d.name)
            .env("APP_FILE", &title)
            .env("APP_ID", &d.bundle_id)
            .env("APP_ICON", &svg)
            .env("OUT_DIR", &out);
        if let Some(v) = &opts.version {
            script.env("VERSION", v);
        }
        run_script(&mut script)?;
        made.push((
            "desktop",
            out.join(if mac {
                format!("{title}-macos-arm64.dmg")
            } else {
                format!("{title}-linux-x86_64.AppImage")
            }),
        ));
    }

    if let Some(bin) = &node
        && wants(Target::NodeImage)
    {
        let d = describe(bin)?;
        let tar = out.join(format!("{}-node.tar", d.folder));
        image(
            "architect-image-prebuilt",
            &[
                ("ARCHITECT_PREBUILT_BIN", bin.as_os_str()),
                ("ARCHITECT_IMAGE_BIN", bin.file_name().unwrap_or_default()),
            ],
            &d.folder,
            &tar,
        )?;
        made.push(("node_image", tar));
    }

    if let Some(bin) = &node
        && (wants(Target::Web) || wants(Target::WebImage))
    {
        let d = describe(bin)?;
        let json = out.join("distribution.json");
        std::fs::write(&json, &d.json).map_err(|e| format!("{}: {e}", json.display()))?;
        let web = out.join("web");
        run_script(
            Command::new("bash")
                .arg(root().join("scripts/web-bundle.sh"))
                .arg(&web)
                .env("ARCHITECT_DISTRIBUTION", &json),
        )?;
        made.push(("web", web.clone()));
        if wants(Target::WebImage) {
            let tar = out.join(format!("{}-web.tar", d.folder));
            image(
                "architect-web-image-prebuilt",
                &[("ARCHITECT_WEB_BUNDLE", web.as_os_str())],
                &format!("{}-web", d.folder),
                &tar,
            )?;
            made.push(("web_image", tar));
        }
    }

    for (k, v) in made {
        println!("{k}={}", v.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_title_makes_a_file_name_without_spaces() {
        assert_eq!(super::file_title("Architect Sandbox"), "ArchitectSandbox");
        assert_eq!(super::file_title("Studio"), "Studio");
    }
}
