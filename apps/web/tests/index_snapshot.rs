//! A large vault's index as the browser receives it (#495, #710): a
//! generated vault of `INDEX_NOTES` notes (10 000 by default; the shape of
//! `features/vault/vault/tests/query_bench.rs`), indexed with the one
//! builder (`vault_query::Builder`), its snapshot's bytes raw and gzipped,
//! and one property edit's delta. With `INDEX_SNAPSHOT_OUT=<file>` the
//! snapshot is written there, for the browser's first-load timing
//! (`apps/web/test/app-index.test.mjs`).
//!
//! `cargo test -p architect-web --release --test index_snapshot -- --ignored --nocapture`

#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use std::io::Write as _;

use vault_query::{Builder, Note, NoteSource};

const STATUSES: &[&str] = &["todo", "doing", "done", "blocked"];

/// Generated notes, `(path, text)`, a revision.
struct Notes(u64, Vec<(String, String)>);

impl NoteSource for Notes {
    fn revision(&self) -> u64 {
        self.0
    }

    fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize) {
        for (path, text) in self.1.iter().skip(from).take(n) {
            each(Note {
                path,
                stamp: {
                    use std::hash::{Hash as _, Hasher as _};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    text.hash(&mut h);
                    h.finish()
                },
                text: vault_query::Text::Here(text),
                parsed: None,
                mtime: 1,
                is_base: false,
            });
        }
        (self.0, self.1.len())
    }
}

fn generate(n: usize) -> Vec<(String, String)> {
    let prose = "Some words about the thing, a [[Link]] or two and a #tag here. ".repeat(12);
    (0..n)
        .map(|i| {
            (
                format!("Area {}/Note {i}.md", i % 20),
                format!(
                    "---\nstatus: {}\npriority: {}\ndue: 2026-{:02}-{:02}\ntags: [note, t{}]\nproject: \"[[Project {}]]\"\n---\n# Note {i}\n\n{prose}\n\n## Tasks\n- [ ] a task for {i}\n- [x] a done one\n",
                    STATUSES[i % STATUSES.len()],
                    i % 5,
                    i % 12 + 1,
                    i % 28 + 1,
                    i % 7,
                    i % 50,
                ),
            )
        })
        .collect()
}

fn gzipped(bytes: &[u8]) -> usize {
    let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
    z.write_all(bytes).unwrap();
    z.finish().unwrap().len()
}

#[test]
#[ignore = "a measurement: run it with --ignored --nocapture, in release"]
fn a_large_vault_s_index_on_the_wire() {
    let n: usize = std::env::var("INDEX_NOTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    let mut notes = Notes(1, generate(n));
    let mut builder = Builder::new();
    let t = std::time::Instant::now();
    let _ = builder.sync(&notes);
    let built = t.elapsed();
    let snapshot = builder.index().to_bytes();
    // One edit: a property set on one note.
    notes.1[17].1 = notes.1[17].1.replacen("priority: 2", "priority: 4", 1);
    notes.0 += 1;
    let delta = builder.sync(&notes).to_bytes();
    let kib = |b: usize| b as f64 / 1024.0;
    println!(
        "{n} notes: built in {:.0} ms; snapshot {:.0} KiB raw ({} B a note), {:.0} KiB gzipped ({} B a note); one property edit's delta {} B raw, {} B gzipped",
        built.as_secs_f64() * 1000.0,
        kib(snapshot.len()),
        snapshot.len() / n,
        kib(gzipped(&snapshot)),
        gzipped(&snapshot) / n,
        delta.len(),
        gzipped(&delta),
    );
    if let Ok(out) = std::env::var("INDEX_SNAPSHOT_OUT") {
        std::fs::write(out, &snapshot).unwrap();
    }
}
