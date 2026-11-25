//! The wire is a published contract: apps already shipped call these
//! services by their vox method ids and their HTTP paths. Every method of
//! every service is pinned here, so a change to a signature, a name or a
//! path fails this test instead of breaking those apps.
//!
//! `BLESS=1 cargo test -p auth-proto --test wire` rewrites
//! `tests/wire.golden` from the code (only when the change is meant).
#![cfg(feature = "vox")]

fn methods(d: &vox::connection::ServiceDescriptor) -> Vec<String> {
    d.methods
        .iter()
        .map(|m| format!("vox {}.{} {:?}", m.service_name, m.method_name, m.id))
        .collect()
}

fn paths(service: &str, p: &[(&str, &str)]) -> Vec<String> {
    p.iter()
        .map(|(m, path)| format!("http {service}.{m} POST {path}"))
        .collect()
}

#[test]
fn the_wire_is_what_shipped_apps_call() {
    let mut lines = Vec::new();
    lines.extend(methods(auth_proto::service::auth_service_service_descriptor()));
    lines.extend(methods(
        auth_proto::organizations::organization_service_service_descriptor(),
    ));
    lines.extend(methods(auth_proto::device::device_service_service_descriptor()));
    lines.extend(paths("AuthService", auth_proto::auth_service_http::PATHS));
    lines.extend(paths(
        "OrganizationService",
        auth_proto::organization_service_http::PATHS,
    ));
    lines.extend(paths("DeviceService", auth_proto::device_service_http::PATHS));
    let now = lines.join("\n") + "\n";
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wire.golden");
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(&golden, &now).unwrap();
        return;
    }
    let pinned = std::fs::read_to_string(&golden).expect("tests/wire.golden");
    assert_eq!(now, pinned, "the wire changed; BLESS=1 only if that is meant");
}
