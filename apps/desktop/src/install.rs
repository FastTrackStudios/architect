//! `architect-desktop install`: this binary, installed as the app on this
//! machine, from a checkout.
//!
//! What a release is, without waiting for one: on a Mac the running binary
//! is bundled as `<Name>.app` and made into a disk image as
//! `scripts/desktop/macos.sh` does, signed with the Developer ID in the
//! build keychain (hardened runtime, the release's entitlements),
//! notarized with the App Store Connect key (`~/.appstoreconnect/config.env`,
//! or `ASC_KEY_ID`, `ASC_ISSUER_ID`, `ASC_KEY_PATH`) and stapled, then put in
//! `/Applications` (`--user`: `~/Applications`) in place of the one before.
//! The disk image is kept (`target/desktop/` from a checkout, `--dmg`
//! elsewhere). With no Developer ID it is signed ad hoc and not notarized;
//! `--no-notarize` skips Apple's round trip. On Linux the binary goes
//! to `~/.local/lib/<app>/` with a desktop entry and its icon. Either way
//! the command line is linked onto `PATH` (`~/.local/bin/architect` → the
//! installed binary), so `architect dev …` works from any folder.
//!
//! ```text
//! cargo run --release -p architect-desktop -- install [--user] [--to <dir>] [--dmg <file>] [--no-notarize] [--no-link] [--link-as <name>]
//! ```

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
// The macOS install shells out (ditto, codesign, sips); Linux copies files.
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};

use architect_distribution::Distribution;

const USAGE: &str = "usage: architect-desktop install [--user] [--to <dir>] [--dmg <file>] [--no-notarize] [--no-link] [--link-as <name>]
  --user          into ~/Applications (Linux: always the person's own)
  --to <dir>      into <dir> instead
  --dmg <file>    where the disk image goes (default: target/desktop/ in a checkout)
  --no-notarize   sign only, nothing sent to Apple (not for handing out)
  --no-link       leave PATH alone
  --link-as <n>   the command's name on PATH (default: the app's, without -desktop)";

/// The build keychain the release script signs from.
#[cfg(target_os = "macos")]
const KEYCHAIN: &str = "fts-build.keychain";

/// The entitlements a Developer ID signature carries (the release's).
#[cfg(target_os = "macos")]
const ENTITLEMENTS: &str = include_str!("../../../scripts/desktop/macos.entitlements");

#[derive(Debug, Default, PartialEq, Eq)]
struct Ask {
    user: bool,
    to: Option<PathBuf>,
    link: bool,
    link_as: Option<String>,
    dmg: Option<PathBuf>,
    notarize: bool,
}

fn parse(args: &[String]) -> Result<Ask, String> {
    let mut ask = Ask {
        link: true,
        notarize: true,
        ..Ask::default()
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--user" => ask.user = true,
            "--no-link" => ask.link = false,
            "--no-notarize" => ask.notarize = false,
            "--dmg" => ask.dmg = Some(it.next().ok_or("--dmg needs a file")?.into()),
            "--to" => ask.to = Some(it.next().ok_or("--to needs a folder")?.into()),
            "--link-as" => {
                let name = it.next().ok_or("--link-as needs a name")?;
                if name.is_empty() || name.contains('/') {
                    return Err(format!("--link-as {name:?}: a plain name"));
                }
                ask.link_as = Some(name.clone());
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(ask)
}

/// The command's name on `PATH`: the app's binary without `-desktop`
/// (`architect-desktop` → `architect`).
fn command_name(app: &str) -> String {
    app.strip_suffix("-desktop").unwrap_or(app).to_owned()
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_owned())
}

fn say(line: impl AsRef<str>) {
    let _ = writeln!(std::io::stdout(), "{}", line.as_ref());
}

// The macOS install shells out (ditto, codesign, sips); Linux copies files.
#[cfg(target_os = "macos")]
fn run(cmd: &mut Command) -> Result<String, String> {
    let shown = format!("{cmd:?}");
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{shown}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{shown}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Runs `install` with `args` (after the subcommand).
#[must_use]
pub fn main(dist: &Distribution, args: &[String]) -> ExitCode {
    if args
        .iter()
        .any(|a| a == "--help" || a == "-h" || a == "help")
    {
        say(USAGE);
        return ExitCode::SUCCESS;
    }
    let result = parse(args)
        .map_err(|why| format!("{why}\n{USAGE}"))
        .and_then(|ask| install(dist, &ask));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            let _ = writeln!(std::io::stderr(), "architect-desktop install: {why}");
            ExitCode::FAILURE
        }
    }
}

fn install(dist: &Distribution, ask: &Ask) -> Result<(), String> {
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| format!("this binary: {e}"))?;
    if cfg!(debug_assertions) {
        say(
            "note: this is a debug build; `cargo run --release -p architect-desktop -- install` installs a release one",
        );
    }
    let installed = place(dist, ask, &exe)?;
    if ask.link {
        link(dist, ask, &installed)?;
    }
    Ok(())
}

/// Links `bin` onto `PATH` as `~/.local/bin/<name>`, replacing a link (or
/// a file) of that name.
fn link(dist: &Distribution, ask: &Ask, bin: &Path) -> Result<(), String> {
    let name = ask
        .link_as
        .clone()
        .unwrap_or_else(|| command_name(dist.app_name()));
    let dir = home()?.join(".local/bin");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let at = dir.join(&name);
    if std::fs::symlink_metadata(&at).is_ok() {
        std::fs::remove_file(&at).map_err(|e| format!("{}: {e}", at.display()))?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(bin, &at).map_err(|e| format!("{}: {e}", at.display()))?;
    say(format!("linked {} → {}", at.display(), bin.display()));
    let on_path =
        std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir));
    if !on_path {
        say(format!(
            "note: {} is not on PATH; add it to run `{name}` from anywhere",
            dir.display()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
/// The SVG icon as a PNG `px` wide: `rsvg-convert` from `PATH`, else from
/// nixpkgs; `None` when neither is there (the app goes without one).
fn render_icon(svg: &str, px: u32, out: &Path) -> Option<()> {
    let src = out.with_extension("svg");
    std::fs::write(&src, svg).ok()?;
    let size = px.to_string();
    let args = ["-w", &size, "-h", &size];
    let direct = Command::new("rsvg-convert")
        .args(args)
        .arg(&src)
        .arg("-o")
        .arg(out)
        .stderr(Stdio::null())
        .status();
    if direct.is_ok_and(|s| s.success()) && out.is_file() {
        return Some(());
    }
    let nix = Command::new("nix")
        .args(["shell", "nixpkgs#librsvg", "-c", "rsvg-convert"])
        .args(args)
        .arg(&src)
        .arg("-o")
        .arg(out)
        .stderr(Stdio::null())
        .status();
    (nix.is_ok_and(|s| s.success()) && out.is_file()).then_some(())
}

#[cfg(target_os = "macos")]
fn place(dist: &Distribution, ask: &Ask, exe: &Path) -> Result<PathBuf, String> {
    let title = dist.name();
    let bin = dist.app_name();
    let folder = match (&ask.to, ask.user) {
        (Some(to), _) => to.clone(),
        (None, true) => home()?.join("Applications"),
        (None, false) => PathBuf::from("/Applications"),
    };
    let stage = std::env::temp_dir().join(format!("{bin}-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    let app = stage.join(format!("{title}.app"));
    let macos = app.join("Contents/MacOS");
    let resources = app.join("Contents/Resources");
    for dir in [&macos, &resources] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let inside = macos.join(bin);
    std::fs::copy(exe, &inside).map_err(|e| format!("{}: {e}", inside.display()))?;
    unnix(&inside)?;
    let icon = bundle_icon(dist, &stage, &resources);
    let plist = info_plist(dist, icon);
    std::fs::write(app.join("Contents/Info.plist"), plist)
        .map_err(|e| format!("Info.plist: {e}"))?;
    let developer = sign(&app, &stage)?;
    disk_image(dist, ask, &app, &stage, developer)?;

    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let target = folder.join(format!("{title}.app"));
    if target.exists() {
        say(format!("replacing {}", target.display()));
        std::fs::remove_dir_all(&target).map_err(|e| {
            format!(
                "{}: {e} (quit it first, or install with --user)",
                target.display()
            )
        })?;
    }
    // `ditto` keeps the signature's extended attributes whole.
    run(Command::new("ditto").arg(&app).arg(&target))?;
    let _ = std::fs::remove_dir_all(&stage);
    say(format!("installed {}", target.display()));
    Ok(target.join("Contents/MacOS").join(bin))
}

/// Points the dev shell's `/nix/store` libiconv and libz at the system's,
/// as the release does; any other store library is refused (no other Mac
/// has it).
#[cfg(target_os = "macos")]
fn unnix(bin: &Path) -> Result<(), String> {
    let linked = run(Command::new("otool").arg("-L").arg(bin))?;
    let store: Vec<&str> = linked
        .lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next())
        .filter(|l| l.starts_with("/nix/store/"))
        .collect();
    for lib in store {
        let system = if lib.ends_with("/libiconv.2.dylib") {
            "/usr/lib/libiconv.2.dylib"
        } else if lib.ends_with("/libz.1.dylib") {
            "/usr/lib/libz.1.dylib"
        } else {
            return Err(format!(
                "{} links {lib} from /nix/store, which no other Mac has",
                bin.display()
            ));
        };
        run(Command::new("install_name_tool")
            .args(["-change", lib, system])
            .arg(bin))?;
    }
    Ok(())
}

/// The `.icns` from the distribution's SVG, in `resources`: its name, or
/// `None` when it could not be made.
#[cfg(target_os = "macos")]
fn bundle_icon(dist: &Distribution, stage: &Path, resources: &Path) -> Option<String> {
    let title = dist.name();
    let png = stage.join("icon-1024.png");
    if render_icon(dist.icons().svg, 1024, &png).is_none() {
        say("note: no rsvg-convert (nor nix) to draw the icon; the app goes without one");
        return None;
    }
    let set = stage.join(format!("{title}.iconset"));
    std::fs::create_dir_all(&set).ok()?;
    for s in [16_u32, 32, 128, 256, 512] {
        for (scale, name) in [
            (1, format!("icon_{s}x{s}.png")),
            (2, format!("icon_{s}x{s}@2x.png")),
        ] {
            let px = s.saturating_mul(scale).to_string();
            run(Command::new("sips")
                .args(["-z", &px, &px])
                .arg(&png)
                .arg("--out")
                .arg(set.join(name)))
            .ok()?;
        }
    }
    run(Command::new("iconutil")
        .args(["-c", "icns"])
        .arg(&set)
        .arg("-o")
        .arg(resources.join(format!("{title}.icns"))))
    .ok()?;
    Some(title.to_owned())
}

#[cfg(target_os = "macos")]
fn info_plist(dist: &Distribution, icon: Option<String>) -> String {
    let (title, bin, version) = (dist.name(), dist.app_name(), dist.version_str());
    let id = format!("{}.mac", dist.id());
    let build = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let icon = icon.map_or_else(String::new, |i| {
        format!("    <key>CFBundleIconFile</key><string>{i}</string>\n")
    });
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>{title}</string>
    <key>CFBundleDisplayName</key><string>{title}</string>
    <key>CFBundleIdentifier</key><string>{id}</string>
    <key>CFBundleExecutable</key><string>{bin}</string>
{icon}    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundleVersion</key><string>{build}</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLName</key><string>{id}.links</string>
            <key>CFBundleURLSchemes</key><array><string>architect</string></array>
        </dict>
    </array>
</dict>
</plist>
"#
    )
}

/// The Developer ID in the build keychain, when it is there and open.
#[cfg(target_os = "macos")]
fn developer_id() -> Option<String> {
    let out =
        run(Command::new("security").args(["find-identity", "-v", "-p", "codesigning", KEYCHAIN]))
            .ok()?;
    out.lines()
        .find(|l| l.contains("Developer ID Application"))
        .and_then(|l| l.split('"').nth(1))
        .map(str::to_owned)
}

/// Signs `app`: the Developer ID with the hardened runtime and the
/// release's entitlements, else ad hoc (this machine runs it either way).
#[cfg(target_os = "macos")]
fn sign(app: &Path, stage: &Path) -> Result<bool, String> {
    if let Some(id) = developer_id() {
        let ent = stage.join("entitlements.plist");
        std::fs::write(&ent, ENTITLEMENTS).map_err(|e| format!("entitlements: {e}"))?;
        let signed = run(Command::new("codesign")
            .args([
                "--force",
                "--keychain",
                KEYCHAIN,
                "--timestamp",
                "--options",
                "runtime",
            ])
            .arg("--entitlements")
            .arg(&ent)
            .args(["--sign", &id])
            .arg(app));
        match signed {
            Ok(_) => {
                say(format!("signed with {id}"));
                return Ok(true);
            }
            Err(why) => say(format!(
                "note: the Developer ID did not sign ({why}); signing ad hoc"
            )),
        }
    } else {
        say(format!(
            "note: no Developer ID in {KEYCHAIN}; signing ad hoc"
        ));
    }
    run(Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(app))
    .map(|_| false)
}

/// The disk image of `app` (beside a link to `/Applications`), signed;
/// with the Developer ID and `ask.notarize`, notarized and stapled, and the
/// app stapled too, so the installed one opens offline.
#[cfg(target_os = "macos")]
fn disk_image(
    dist: &Distribution,
    ask: &Ask,
    app: &Path,
    stage: &Path,
    developer: bool,
) -> Result<(), String> {
    let title = dist.name();
    let dmg = ask.dmg.clone().unwrap_or_else(|| {
        let checkout = std::env::current_dir()
            .ok()
            .filter(|d| d.join("Cargo.toml").is_file());
        checkout
            .map_or_else(|| stage.join("out"), |d| d.join("target/desktop"))
            .join(format!("{title}-macos-arm64.dmg"))
    });
    if let Some(dir) = dmg.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let root = stage.join("dmg");
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    run(Command::new("ditto")
        .arg(app)
        .arg(root.join(format!("{title}.app"))))?;
    std::os::unix::fs::symlink("/Applications", root.join("Applications"))
        .map_err(|e| format!("the Applications link: {e}"))?;
    let _ = std::fs::remove_file(&dmg);
    let volume = format!("{title} {}", dist.version_str());
    run(Command::new("hdiutil")
        .args(["create", "-volname", &volume, "-srcfolder"])
        .arg(&root)
        .args(["-ov", "-format", "UDZO"])
        .arg(&dmg))?;
    let signer = developer.then(developer_id).flatten();
    let Some(id) = signer else {
        say(format!(
            "disk image {} (ad hoc: not notarized)",
            dmg.display()
        ));
        return Ok(());
    };
    // Gatekeeper checks the image a person opens, not only the app in it.
    run(Command::new("codesign")
        .args([
            "--force",
            "--keychain",
            KEYCHAIN,
            "--timestamp",
            "--sign",
            &id,
        ])
        .arg(&dmg))?;
    if !ask.notarize {
        say(format!(
            "disk image {} (signed, not notarized: --no-notarize)",
            dmg.display()
        ));
        return Ok(());
    }
    let Some(key) = Notary::find() else {
        say(format!(
            "note: no App Store Connect key (~/.appstoreconnect/config.env, or ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH): disk image {} signed, not notarized",
            dmg.display()
        ));
        return Ok(());
    };
    say("notarizing: Apple takes a few minutes");
    let log = run(Command::new("xcrun")
        .args(["notarytool", "submit"])
        .arg(&dmg)
        .args([
            "--key",
            &key.path,
            "--key-id",
            &key.id,
            "--issuer",
            &key.issuer,
            "--wait",
        ]))?;
    if !log.contains("status: Accepted") {
        return Err(format!("notarization was not accepted:\n{}", log.trim()));
    }
    run(Command::new("xcrun").args(["stapler", "staple"]).arg(&dmg))?;
    run(Command::new("xcrun").args(["stapler", "staple"]).arg(app))?;
    say(format!("notarized and stapled: {}", dmg.display()));
    let gatekeeper = Command::new("spctl")
        .args(["-a", "-t", "exec", "-vv"])
        .arg(app)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stderr).trim().to_owned())
        .unwrap_or_default();
    if !gatekeeper.is_empty() {
        say(format!("Gatekeeper: {}", gatekeeper.replace('\n', "; ")));
    }
    Ok(())
}

/// The App Store Connect API key notarytool signs in with.
#[cfg(target_os = "macos")]
struct Notary {
    id: String,
    issuer: String,
    path: String,
}

#[cfg(target_os = "macos")]
impl Notary {
    /// From `ASC_KEY_ID`, `ASC_ISSUER_ID` and `ASC_KEY_PATH`, else from
    /// `~/.appstoreconnect/config.env` (the release script's): `KEY=value`
    /// lines, `export` and quotes allowed.
    fn find() -> Option<Self> {
        let file: std::collections::BTreeMap<String, String> = home()
            .ok()
            .and_then(|h| std::fs::read_to_string(h.join(".appstoreconnect/config.env")).ok())
            .map(|text| {
                text.lines()
                    .filter_map(|l| {
                        let l = l.trim().trim_start_matches("export ").trim();
                        let (k, v) = l.split_once('=')?;
                        Some((
                            k.trim().to_owned(),
                            v.trim().trim_matches(['"', '\'']).to_owned(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let get = |k: &str| {
            std::env::var(k)
                .ok()
                .filter(|v| !v.is_empty())
                .or_else(|| file.get(k).cloned())
                .map(|v| expand_home(&v))
        };
        Some(Self {
            id: get("ASC_KEY_ID")?,
            issuer: get("ASC_ISSUER_ID")?,
            path: get("ASC_KEY_PATH")?,
        })
    }
}

/// `~/…`, `$HOME/…` and `${HOME}/…` as the home folder.
#[cfg(target_os = "macos")]
fn expand_home(v: &str) -> String {
    let Ok(home) = home() else {
        return v.to_owned();
    };
    let home = home.display().to_string();
    v.strip_prefix("~/")
        .map(|rest| format!("{home}/{rest}"))
        .or_else(|| {
            v.strip_prefix("${HOME}")
                .map(|rest| format!("{home}{rest}"))
        })
        .or_else(|| v.strip_prefix("$HOME").map(|rest| format!("{home}{rest}")))
        .unwrap_or_else(|| v.to_owned())
}

#[cfg(not(target_os = "macos"))]
fn place(dist: &Distribution, ask: &Ask, exe: &Path) -> Result<PathBuf, String> {
    let home = home()?;
    let bin = dist.app_name();
    let root = ask
        .to
        .clone()
        .unwrap_or_else(|| home.join(".local/lib").join(bin));
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    // Beside the old one, then over it: a running copy keeps its file.
    let installed = root.join(bin);
    let fresh = root.join(format!(".{bin}.new"));
    std::fs::copy(exe, &fresh).map_err(|e| format!("{}: {e}", fresh.display()))?;
    std::fs::rename(&fresh, &installed).map_err(|e| format!("{}: {e}", installed.display()))?;
    say(format!("installed {}", installed.display()));

    let id = dist.id();
    let icons = home.join(".local/share/icons/hicolor/scalable/apps");
    let apps = home.join(".local/share/applications");
    for dir in [&icons, &apps] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let icon = icons.join(format!("{id}.svg"));
    std::fs::write(&icon, dist.icons().svg).map_err(|e| format!("{}: {e}", icon.display()))?;
    let entry = apps.join(format!("{id}.desktop"));
    std::fs::write(
        &entry,
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={exec} %u\nIcon={id}\n\
             Terminal=false\nCategories=Office;\nMimeType=x-scheme-handler/architect;\nStartupWMClass={bin}\n",
            name = dist.name(),
            exec = installed.display(),
        ),
    )
    .map_err(|e| format!("{}: {e}", entry.display()))?;
    say(format!("desktop entry {}", entry.display()));
    if ask.user {
        say("note: on Linux it is always the person's own (~/.local)");
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn its_arguments_read_as_asked() {
        let ask = parse(&args("--user --to /tmp/apps --link-as arch")).unwrap();
        assert!(ask.user && ask.link);
        assert_eq!(ask.to.as_deref(), Some(Path::new("/tmp/apps")));
        assert_eq!(ask.link_as.as_deref(), Some("arch"));
        assert!(!parse(&args("--no-link")).unwrap().link);
        let ask = parse(&args("--no-notarize --dmg /tmp/a.dmg")).unwrap();
        assert!(!ask.notarize && parse(&args("")).unwrap().notarize);
        assert_eq!(ask.dmg.as_deref(), Some(Path::new("/tmp/a.dmg")));
        assert!(parse(&args("--to")).is_err());
        assert!(parse(&args("--link-as a/b")).is_err());
        assert!(parse(&args("--what")).is_err());
    }

    #[test]
    fn the_command_is_the_app_without_desktop() {
        assert_eq!(command_name("architect-desktop"), "architect");
        assert_eq!(command_name("keyflow-architect"), "keyflow-architect");
    }
}
