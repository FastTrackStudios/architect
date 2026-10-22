//! `architect::settings::Store<T>`: round-trip, skip-if-equal, 0600, and
//! crash safety (a torn temp file from a dead writer never wins).

use architect::facet::{self, Facet};
use architect::settings::{FileKv, SettingsError, Store, write_atomic};

#[derive(Facet, Debug, Default, Clone, PartialEq)]
struct Prefs {
    theme: String,
    zoom: u32,
    recent: Vec<String>,
    token: Option<String>,
}

fn sample() -> Prefs {
    Prefs {
        theme: "dark".into(),
        zoom: 125,
        recent: vec!["a.song".into(), "b \"quoted\" ✓".into()],
        token: Some("t".into()),
    }
}

#[test]
fn missing_file_loads_default() {
    let dir = tempfile::tempdir().unwrap();
    let s: Store<Prefs> = Store::in_dir("app", "prefs", dir.path());
    assert_eq!(s.load().unwrap(), Prefs::default());
    assert!(!s.path().exists(), "load must not create the file");
}

#[test]
fn round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let s: Store<Prefs> = Store::in_dir("app", "prefs", dir.path().join("nested/app"));
    assert!(s.save(&sample()).unwrap());
    assert_eq!(s.load().unwrap(), sample());
    // A second handle on the same dir reads the same value.
    let again: Store<Prefs> = Store::in_dir("app", "prefs", dir.path().join("nested/app"));
    assert_eq!(again.load().unwrap(), sample());

    let updated = s.update(|p| p.zoom = 90).unwrap();
    assert_eq!(updated.zoom, 90);
    assert_eq!(s.load().unwrap().zoom, 90);

    s.clear().unwrap();
    assert_eq!(s.load().unwrap(), Prefs::default());
    s.clear().unwrap(); // clearing nothing is fine
}

#[test]
fn equal_write_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let s: Store<Prefs> = Store::in_dir("app", "prefs", dir.path());
    assert!(s.save(&sample()).unwrap());
    let before = std::fs::metadata(s.path()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(!s.save(&sample()).unwrap(), "unchanged value must not be rewritten");
    let after = std::fs::metadata(s.path()).unwrap();
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(before.ino(), after.ino(), "no rename happened");
    }
}

#[test]
fn bad_body_is_an_error_not_a_reset() {
    let dir = tempfile::tempdir().unwrap();
    let s: Store<Prefs> = Store::in_dir("app", "prefs", dir.path());
    std::fs::write(s.path(), "{ not json").unwrap();
    assert!(matches!(s.load(), Err(SettingsError::Decode { .. })));
    assert_eq!(std::fs::read_to_string(s.path()).unwrap(), "{ not json");
}

#[test]
fn crash_mid_write_keeps_the_old_value() {
    let dir = tempfile::tempdir().unwrap();
    let s: Store<Prefs> = Store::in_dir("app", "prefs", dir.path());
    s.save(&sample()).unwrap();

    // A writer that died after writing half its temp file and before the
    // rename: exactly what write_atomic leaves behind on a crash.
    let torn = dir.path().join(".prefs.json.tmp-99999-0");
    std::fs::write(&torn, "{\"theme\":\"li").unwrap();
    assert_eq!(s.load().unwrap(), sample(), "torn temp file is never read");

    // The next save still lands, atomically, and leaves no temp of its own.
    let mut next = sample();
    next.theme = "light".into();
    assert!(s.save(&next).unwrap());
    assert_eq!(s.load().unwrap(), next);
    let temps: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp-") && !n.contains("99999"))
        .collect();
    assert!(temps.is_empty(), "leftover temp files: {temps:?}");
}

#[cfg(unix)]
#[test]
fn failed_rename_leaves_target_and_no_temp() {
    // Target path is a directory: the rename fails after the temp file
    // is fully written. The target must be untouched and the temp gone.
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("prefs.json");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep"), "x").unwrap();
    assert!(write_atomic(&target, b"{}", false).is_err());
    assert!(target.join("keep").exists());
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["prefs.json".to_owned()]);
}

#[cfg(unix)]
#[test]
fn private_files_are_0600() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let s: Store<Prefs> = Store::in_dir("app", "secrets", dir.path()).private();
    s.save(&sample()).unwrap();
    let mode = std::fs::metadata(s.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    // An existing world-readable file is tightened even when the body is equal.
    std::fs::set_permissions(s.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!s.save(&sample()).unwrap());
    let mode = std::fs::metadata(s.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    // Non-private files keep the umask default.
    let kv = FileKv::new(dir.path());
    kv.write("plain", "x").unwrap();
    let mode = std::fs::metadata(kv.path("plain")).unwrap().permissions().mode() & 0o777;
    assert_ne!(mode, 0o600);
}

#[test]
fn open_honours_the_env_override() {
    // The only test touching this variable, so no other test races it.
    let dir = tempfile::tempdir().unwrap();
    let var = architect::settings::env_var("sta390-settings-test");
    assert_eq!(var, "STA390_SETTINGS_TEST_CONFIG_DIR");
    // SAFETY: set before any other thread reads this variable; it is unique to this test.
    unsafe { std::env::set_var(&var, dir.path()) };
    let s: Store<Prefs> = Store::open("sta390-settings-test", "prefs").unwrap();
    assert_eq!(s.path(), dir.path().join("prefs.json"));
    s.save(&sample()).unwrap();
    assert!(dir.path().join("prefs.json").is_file());
}
