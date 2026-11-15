//! Exercise the `VaultSync::base_views` backend method against the live
//! vault — proves the server-side base renderer (row projection, cells,
//! grouping) before the UI consumes it.
//!
//! Run: cargo run -p vault-live --example run_base_views -- [VAULT_ROOT] [BASE_PATH]

// Moved from Task as it was (#333); see the crate root's lint note.
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::iter_on_single_items,
    clippy::unwrap_used
)]

use std::path::PathBuf;

use vault_live::Backend;
use vault_proto::VaultSync;

fn main() {
    let mut args = std::env::args().skip(1);
    let root: PathBuf = args.next().map_or_else(
        || PathBuf::from(std::env::var("HOME").unwrap()).join(".task/orgs/codywright/vault"),
        PathBuf::from,
    );
    let base = args
        .next()
        .unwrap_or_else(|| "Scripture/Songs.base".to_string());

    let backend = Backend::single("default", root).expect("open backend");
    let views = backend.base_views("default", &base).expect("base_views");

    println!("base: {base} — {} view(s)\n", views.len());
    for v in views {
        println!("▸ {} [{}]  columns: {:?}", v.name, v.view_type, v.columns);
        for g in v.groups {
            let label = if g.label.is_empty() {
                "—".into()
            } else {
                g.label
            };
            for r in g.rows {
                println!("    [{label}] {}  ::  {:?}", r.title, r.cells);
            }
        }
        println!();
    }
}
