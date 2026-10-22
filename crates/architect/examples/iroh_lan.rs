//! Manual check for LAN peer discovery: two machines on one network,
//! internet unplugged, dial each other by bare endpoint id.
//!
//! ```sh
//! # machine A
//! cargo run -p architect --example iroh_lan --features iroh-lan -- serve
//! #   prints: endpoint id: <ID>
//! # machine B (same LAN; pull the WAN cable / disable the default route)
//! cargo run -p architect --example iroh_lan --features iroh-lan -- dial <ID>
//! #   prints: vox handshake with <ID> ok
//! ```
//!
//! Both sides bind `PeerLookup::Lan` (relays off, nothing published to
//! n0), so a successful dial can only have come from the local-network
//! lookup. Pass `--n0-and-lan` as the last argument to try the mixed
//! mode a host would normally run.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;
use std::time::Duration;

use architect::iroh_link::{self, PeerLookup};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let lookup = if args.iter().any(|a| a == "--n0-and-lan") {
        PeerLookup::N0AndLan
    } else {
        PeerLookup::Lan
    };
    let result = match args.first().map(String::as_str) {
        Some("serve") => serve(lookup).await,
        Some("dial") => match args.get(1) {
            Some(id) => dial(lookup, id).await,
            None => Err("usage: iroh_lan dial <endpoint-id>".to_owned()),
        },
        _ => Err("usage: iroh_lan serve | dial <endpoint-id> [--n0-and-lan]".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(lookup: PeerLookup) -> Result<(), String> {
    let endpoint = iroh_link::bind_endpoint_with(iroh::SecretKey::generate(), lookup)
        .await
        .map_err(|e| format!("bind: {e}"))?;
    println!("endpoint id: {}", endpoint.id());
    let acceptor = iroh_link::lane_acceptor_fn(|_req, _connection| {
        Err(vox_core::LaneRejection::new(
            vox_core::LaneRejectReason::UnknownService,
        ))
    });
    iroh_link::serve_endpoint(&endpoint, acceptor).await;
    Ok(())
}

async fn dial(lookup: PeerLookup, id: &str) -> Result<(), String> {
    let remote: iroh::EndpointId = id.parse().map_err(|e| format!("endpoint id: {e}"))?;
    let endpoint = iroh_link::bind_endpoint_with(iroh::SecretKey::generate(), lookup)
        .await
        .map_err(|e| format!("bind: {e}"))?;
    let link = tokio::time::timeout(
        Duration::from_secs(30),
        iroh_link::connect(&endpoint, remote),
    )
    .await
    .map_err(|_| "lookup + dial timed out after 30 s".to_owned())?
    .map_err(|e| format!("dial: {e}"))?;
    let connection = vox_core::initiator_on(link)
        .establish_connection()
        .await
        .map_err(|e| format!("vox handshake: {e}"))?;
    drop(connection);
    println!("vox handshake with {remote} ok");
    endpoint.close().await;
    Ok(())
}
