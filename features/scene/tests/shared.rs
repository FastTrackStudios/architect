//! A table in shared memory (`engine::shared`): an engine in another
//! process writes what the host reads, and reads what the host's drags
//! write. Two mappings of one file in this test share their pages exactly
//! as two processes' do.

#![cfg(all(unix, not(target_family = "wasm")))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp
)]

use architect_scene::Error;
use architect_scene::anim::{Name, Space, Values};
use architect_scene::engine::shared;

fn name(n: &str) -> Name {
    Name::new(n).unwrap()
}

#[test]
fn the_engine_writes_what_the_host_reads_and_reads_what_its_drags_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daw.values");
    let (_, writer) = shared::create(
        &path,
        [
            name("engine.daw.track.0.peak"),
            name("host.daw.track.0.volume"),
        ],
    )
    .unwrap();
    let peak = writer
        .table()
        .slot(&name("engine.daw.track.0.peak"))
        .unwrap();
    let volume = writer
        .table()
        .slot(&name("host.daw.track.0.volume"))
        .unwrap();

    // The host maps the same file.
    let host = shared::open(&path).unwrap();
    assert_eq!(host.read(&name("engine.daw.track.0.peak")), None);
    let before = host.generation();
    writer.write(peak, 0.5);
    assert_eq!(host.read(&name("engine.daw.track.0.peak")), Some(0.5));
    assert!(host.generation() > before, "a write moves the generation");

    // A drag on a pane reading the table: the host writes its value, the
    // engine reads it.
    let mut values = Values::new().with_engine(host.clone());
    assert_eq!(writer.read(volume), None);
    values
        .write(Space::Host, &name("host.daw.track.0.volume"), 0.8)
        .unwrap();
    assert_eq!(writer.read(volume), Some(0.8));
    // One the table does not list stays the pane's.
    values
        .write(Space::Host, &name("host.daw.track.9.volume"), 0.1)
        .unwrap();
    assert_eq!(values.get(&name("host.daw.track.9.volume")), Some(0.1));
    // The host never writes the engine's names, nor does a view.
    assert!(!host.write_host(&name("engine.daw.track.0.peak"), 1.0));
    assert_eq!(host.read(&name("engine.daw.track.0.peak")), Some(0.5));
}

#[test]
fn a_restarted_engine_is_found_and_the_old_mapping_stays_whole() {
    // `open_engine` reads $XDG_RUNTIME_DIR; this test gives it its own.
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: no other thread of this test binary reads the environment
    // at this moment (this is the only test touching it).
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", dir.path()) };
    let path = shared::path_of("lab.engine").unwrap();
    assert!(
        shared::open_engine("lab.engine").is_none(),
        "not running yet"
    );

    let (_, first) = shared::create(&path, [name("engine.meter")]).unwrap();
    first.write(first.table().slot(&name("engine.meter")).unwrap(), 0.25);
    let old = shared::open_engine("lab.engine").unwrap();
    assert_eq!(old.read(&name("engine.meter")), Some(0.25));
    // Asked again with nothing new: the same mapping.
    assert!(std::sync::Arc::ptr_eq(
        &old,
        &shared::open_engine("lab.engine").unwrap()
    ));

    // The engine restarts with another track: a new file in its place.
    let (_, second) =
        shared::create(&path, [name("engine.meter"), name("engine.meter.2")]).unwrap();
    second.write(second.table().slot(&name("engine.meter.2")).unwrap(), 0.75);
    let new = shared::open_engine("lab.engine").unwrap();
    assert_eq!(new.read(&name("engine.meter.2")), Some(0.75));
    assert_eq!(new.read(&name("engine.meter")), None);
    // The old mapping still reads what it held (its file was never truncated).
    assert_eq!(old.read(&name("engine.meter")), Some(0.25));
}

#[test]
fn a_file_that_is_not_a_whole_table_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.values");
    shared::create(&path, [name("engine.a"), name("host.b")]).unwrap();
    let good = std::fs::read(&path).unwrap();

    let refused = |bytes: &[u8]| {
        std::fs::write(&path, bytes).unwrap();
        shared::open(&path).unwrap_err()
    };
    let mut magic = good.clone();
    magic[0] = b'X';
    assert!(matches!(refused(&magic), Error::NotAFragment));
    assert!(matches!(refused(&good[..good.len() - 1]), Error::Truncated));
    assert!(matches!(refused(&good[..16]), Error::Truncated));
    let mut many = good.clone();
    let past = u64::try_from(architect_scene::engine::MAX_SLOTS + 1).unwrap();
    many[8..16].copy_from_slice(&past.to_le_bytes());
    assert!(matches!(refused(&many), Error::TooManyValues(_)));
    // A view's name in it.
    let mut view = good;
    let at = view.windows(6).position(|w| w == b"host.b").unwrap();
    view[at..at + 4].copy_from_slice(b"view");
    assert!(matches!(refused(&view), Error::Denied(..)));
    // A view's name can't be made one either.
    assert!(shared::create(&path, [name("view.x")]).is_err());
    assert!(shared::path_of("../etc").is_err());
}

/// A full table (`MAX_SLOTS`, #891) is made, opened from its file and
/// read where it was written.
#[test]
fn a_full_table_crosses_a_shared_file() {
    use architect_scene::engine::MAX_SLOTS;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("full.values");
    let names: Vec<Name> = (0..MAX_SLOTS)
        .map(|i| name(&format!("engine.v{i}")))
        .collect();
    let (table, writer) = shared::create(&path, names.clone()).unwrap();
    let last = names.last().unwrap();
    writer.write(table.slot(last).unwrap(), 0.5);
    let opened = shared::open(&path).unwrap();
    assert_eq!(opened.read(last), Some(0.5));
    assert_eq!(opened.read(&names[0]), None);
}
