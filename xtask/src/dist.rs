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
//! | `ios` | `apps/mobile/ios/build-app.sh --device` on a Mac: the phone's app, unsigned (`apps/mobile/ios/deploy-testflight.sh` signs it) | `ios/<App>.app` |
//! | `ios-sim` | the same for the Simulator, signed ad hoc | `ios-sim/<App>.app` |
//!
//! `ios` and `ios-sim` take a crate whose `main` on iOS is
//! `architect_mobile::run(<its Distribution>)` (`dist/standard`'s is), and
//! are not part of `all`: they need a Mac with Xcode. The app's identity
//! is `apps/mobile/ios/Info.plist`'s (the Architect app's) unless
//! `INFO_PLIST` names another: an iOS binary cannot be asked who it is on
//! the Mac that built it.
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
    /// The iPhone and iPad app (a Mac only; not in `all`).
    Ios,
    /// The same for the iOS Simulator (a Mac only; not in `all`).
    IosSim,
    /// Every target but the iOS ones.
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
    /// The version written into the desktop entry and Info.plist, and
    /// stamped into the binary (`ARCHITECT_RELEASE_VERSION`).
    pub version: Option<String>,
    /// Features of the distribution crate to turn on (`--manifest-path`
    /// only): the standard distribution's node takes `node-wasi`.
    pub features: Vec<String>,
    /// WASI component extensions in the desktop and iOS apps (#707): the
    /// crate's `wasi` feature (Wasmtime; Cranelift on the desk, Pulley on
    /// iOS), when it has one. `--no-wasi` leaves it out.
    pub wasi: bool,
}

/// The build-time env `architect-updates` reads as the app's release
/// version (`RELEASE_VERSION`).
const RELEASE_VERSION_ENV: &str = "ARCHITECT_RELEASE_VERSION";

/// The version to stamp, a leading `v` dropped: exactly `X.Y.Z`, or a
/// nightly's `X.Y.Z-nightly.YYYYMMDD` (the forms the updater and the Nix
/// launcher, `nix/desktop/choose.sh`, rank). Anything else would build an
/// app that never updates, so it is refused.
fn stamp(version: &str) -> Result<String, String> {
    let v = version.trim().trim_start_matches('v');
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let (core, tail) = v.split_once('-').map_or((v, None), |(c, t)| (c, Some(t)));
    let core_ok = {
        let parts: Vec<&str> = core.split('.').collect();
        parts.len() == 3 && parts.iter().all(|p| digits(p))
    };
    let tail_ok = tail.is_none_or(|t| {
        t.strip_prefix("nightly.")
            .is_some_and(|d| d.len() == 8 && digits(d))
    });
    if core_ok && tail_ok {
        Ok(v.to_owned())
    } else {
        Err(format!(
            "--version {version}: give X.Y.Z or X.Y.Z-nightly.YYYYMMDD; any other form builds an app that never updates"
        ))
    }
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
    // The release version, stamped into the binary for its updater
    // (`architect-updates` `RELEASE_VERSION`); without `--version` the
    // build is a development build, which never updates itself.
    if let Some(v) = &opts.version {
        cmd.env(RELEASE_VERSION_ENV, stamp(v)?);
    } else {
        cmd.env_remove(RELEASE_VERSION_ENV);
    }
    let features = if node {
        given_features(opts)
    } else {
        app_features(opts)?
    };
    if !features.is_empty() {
        cmd.args(["--features", &features.join(",")]);
    }
    if node {
        // Without the window (no Blitz, wgpu or wayland), and without the
        // workspace-hack, which would pull the GUI stack's features back
        // in: the release's node build.
        cmd.arg("--no-default-features");
        if opts.manifest_path.is_none() {
            // A distribution crate depends on architect-desktop with
            // `vox` (its node's services) itself. `node-wasi`: installed
            // WASI components draw a share's blocks (#488).
            cmd.args(["--features", "vox,node-wasi"]);
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

/// The distribution crate's binary for iOS: `--bin`, else the crate's
/// only binary (`architect-mobile` for this repo).
fn ios_bin(opts: &Options) -> Result<Option<String>, String> {
    if let Some(bin) = &opts.bin {
        return Ok(Some(bin.clone()));
    }
    let Some(manifest) = &opts.manifest_path else {
        return Ok(None);
    };
    let manifest = manifest
        .canonicalize()
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    let meta = metadata(&manifest)?;
    let bins = only_bins(&meta, &manifest);
    match bins.as_slice() {
        [one] => Ok(Some(one.clone())),
        [] => Err(format!("{} has no binary", manifest.display())),
        _ => Err(format!(
            "{} has several binaries ({}): name one with --bin",
            manifest.display(),
            bins.join(", ")
        )),
    }
}

/// The features `--features` named, for a distribution crate
/// (`--manifest-path`); this repo's own crates take none.
fn given_features(opts: &Options) -> Vec<String> {
    if opts.manifest_path.is_some() {
        opts.features.clone()
    } else {
        Vec::new()
    }
}

/// The features of the app's build, the desk's window or iOS: those
/// given, and `wasi` (#707) unless `--no-wasi`, when the crate has it
/// (this repo's `architect-desktop` and `architect-mobile` do).
fn app_features(opts: &Options) -> Result<Vec<String>, String> {
    let mut features = given_features(opts);
    if !opts.wasi || features.iter().any(|f| f == "wasi") {
        return Ok(features);
    }
    let has = match &opts.manifest_path {
        None => true,
        Some(m) => {
            let m = m
                .canonicalize()
                .map_err(|e| format!("{}: {e}", m.display()))?;
            declares_feature(&metadata(&m)?, &m, "wasi")
        }
    };
    if has {
        features.push("wasi".to_owned());
    } else {
        eprintln!("dist: the crate has no `wasi` feature: installed WASI extensions will not run");
    }
    Ok(features)
}

/// Whether the package whose manifest is `manifest` declares `feature`,
/// in `cargo metadata`'s answer.
fn declares_feature(meta: &serde_json::Value, manifest: &Path, feature: &str) -> bool {
    meta.get("packages")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter(|p| {
            p.get("manifest_path")
                .and_then(|m| m.as_str())
                .is_some_and(|m| Path::new(m) == manifest)
        })
        .any(|p| p.get("features").and_then(|f| f.get(feature)).is_some())
}

/// `cargo metadata --no-deps` of the crate at `manifest`.
fn metadata(manifest: &Path) -> Result<serde_json::Value, String> {
    let out = Command::new(cargo())
        .current_dir(root())
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata --manifest-path {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))
}

/// The bin targets of the package whose manifest is `manifest`, in
/// `cargo metadata`'s answer.
fn only_bins(meta: &serde_json::Value, manifest: &Path) -> Vec<String> {
    let packages = meta.get("packages").and_then(|p| p.as_array());
    packages
        .into_iter()
        .flatten()
        .filter(|p| {
            p.get("manifest_path")
                .and_then(|m| m.as_str())
                .is_some_and(|m| Path::new(m) == manifest)
        })
        .flat_map(|p| {
            p.get("targets")
                .and_then(|t| t.as_array())
                .into_iter()
                .flatten()
        })
        .filter(|t| {
            t.get("kind")
                .and_then(|k| k.as_array())
                .is_some_and(|k| k.iter().any(|k| k == "bin"))
        })
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_owned))
        .collect()
}

/// The iOS app of the distribution (`device`: the phone's, unsigned;
/// else the Simulator's, signed ad hoc), built by
/// `apps/mobile/ios/build-app.sh` and copied into `out/<ios|ios-sim>/`.
fn ios(opts: &Options, device: bool, out: &Path) -> Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        return Err("dist ios builds on a Mac with Xcode (airlock)".to_owned());
    }
    let mut cmd = Command::new("bash");
    cmd.arg(root().join("apps/mobile/ios/build-app.sh"));
    if device {
        cmd.arg("--device");
    }
    cmd.args(["--profile", &opts.profile]);
    if let Some(m) = &opts.manifest_path {
        let m = m
            .canonicalize()
            .map_err(|e| format!("{}: {e}", m.display()))?;
        cmd.arg("--manifest-path").arg(m);
    }
    if let Some(bin) = ios_bin(opts)? {
        cmd.args(["--bin", &bin]);
    }
    let features = app_features(opts)?;
    if !features.is_empty() {
        cmd.args(["--features", &features.join(",")]);
    }
    if let Some(dir) = &opts.target_dir {
        cmd.env("CARGO_TARGET_DIR", dir);
    }
    if let Some(v) = &opts.version {
        cmd.env(RELEASE_VERSION_ENV, stamp(v)?);
    } else {
        cmd.env_remove(RELEASE_VERSION_ENV);
    }
    eprintln!("dist: {cmd:?}");
    let built = cmd
        .current_dir(root())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("build-app.sh: {e}"))?;
    if !built.status.success() {
        return Err("apps/mobile/ios/build-app.sh failed".to_owned());
    }
    let app = String::from_utf8_lossy(&built.stdout)
        .lines()
        .last()
        .map(|l| PathBuf::from(l.trim()))
        .filter(|p| p.is_dir())
        .ok_or_else(|| "build-app.sh named no app bundle".to_owned())?;
    let dir = out.join(if device { "ios" } else { "ios-sim" });
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let copy = dir.join(app.file_name().unwrap_or_default());
    if copy.exists() {
        std::fs::remove_dir_all(&copy).map_err(|e| format!("{}: {e}", copy.display()))?;
    }
    // ditto: the bundle's symlinks and the ad hoc signature kept.
    run_script(Command::new("ditto").arg(&app).arg(&copy))?;
    Ok(copy)
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

    // The iOS apps: never in `all` (a Mac with Xcode only).
    if opts.targets.contains(&Target::IosSim) {
        made.push(("ios_sim", ios(opts, false, &out)?));
    }
    if opts.targets.contains(&Target::Ios) {
        made.push(("ios", ios(opts, true, &out)?));
    }

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
    fn the_ios_binary_is_the_crate_s_own() {
        let meta = serde_json::json!({ "packages": [
            { "manifest_path": "/a/Cargo.toml", "targets": [
                { "name": "standard", "kind": ["lib"] },
                { "name": "architect-desktop", "kind": ["bin"] },
            ] },
            { "manifest_path": "/b/Cargo.toml", "targets": [
                { "name": "other", "kind": ["bin"] },
            ] },
        ] });
        assert_eq!(
            super::only_bins(&meta, std::path::Path::new("/a/Cargo.toml")),
            ["architect-desktop"]
        );
        assert!(super::only_bins(&meta, std::path::Path::new("/c/Cargo.toml")).is_empty());
    }

    #[test]
    fn a_title_makes_a_file_name_without_spaces() {
        assert_eq!(super::file_title("Architect Sandbox"), "ArchitectSandbox");
        assert_eq!(super::file_title("Studio"), "Studio");
    }

    #[test]
    fn only_a_release_or_a_nightly_version_is_stamped() {
        assert_eq!(super::stamp("v0.13.0").as_deref(), Ok("0.13.0"));
        assert_eq!(
            super::stamp("0.13.0-nightly.20261111").as_deref(),
            Ok("0.13.0-nightly.20261111")
        );
        for bad in ["0.13.0-rc.1", "0.13", "0.13.0-nightly.2026111", "main", ""] {
            assert!(super::stamp(bad).is_err(), "{bad}");
        }
    }
}
