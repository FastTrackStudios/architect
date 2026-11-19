// Import debt (#512). This crate is vendored mermaid-rs (upstream's layout engine and
// renderer, moved in with the editor from FastTrackStudios/editor). It is under the
// workspace lints like every member, and this crate-level list is what
// keeps it building there: 3,411 lint errors in the library
// alone on the day it moved in, almost all indexing, arithmetic and casts.
// Do not add to it for code written here; it goes when the crate is
// replaced or brought up to the house lints.
#![allow(
    clippy::pedantic,
    clippy::nursery,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::unimplemented,
    clippy::unchecked_time_subtraction,
    clippy::todo,
    clippy::string_slice,
    clippy::panic_in_result_fn,
    clippy::panic,
    clippy::exit,
    clippy::as_conversions
)]
fn main() {
    if let Err(err) = mermaid_rs_renderer::run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
