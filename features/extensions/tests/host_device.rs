//! The device's services lent to extensions (#709): OS notifications, the
//! camera, the photo library and health data, on a host whose device is a
//! `FakeDevice`. A Luau extension makes each call through `host.call` and
//! gets the answers back; a capture or a photo's copy is a file it may
//! then read; each call is refused outside its `capabilities.device`
//! entry (health type by type, read and write apart); a host with no
//! device says it cannot; a manifest naming an unknown entry does not
//! load; consent lists each entry and a grant narrows them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use architect::action::{ActionCaller, ActionList, Principal};
use architect_extensions::device::{DeviceCall, FakeDevice, PhotoAsset};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::install::Installs;
use architect_extensions::manifest::{Capabilities, Manifest};
use architect_extensions::package::Package;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const MANIFEST: &str = r#"
id = "lab.meals"
name = "Meals"
version = "0.1.0"
kind = "luau"
api = "0.1"

[capabilities]
device = ["notifications", "camera", "photos", "health:read:steps", "health:write:dietary_energy"]

[[actions]]
id = "LAB_MEALS_LOG"
name = "Log a meal"
script = "log.luau"
"#;

/// The meal plan's day: a photo of the meal, its calories to Health, a
/// reminder for the next one, today's steps.
const LOG: &str = r#"
local photo = host.call({ op = "capture" })
local wrote = host.call({ op = "health_write", type = "dietary_energy", value = 640, start_ms = 1000 })
local reminder = host.call({ op = "os_notify", id = "dinner", title = "Dinner", body = "Log it", after_s = 3600 })
local steps = host.call({ op = "health_read", type = "steps", from_ms = 0, to_ms = 2000 })
return { photo = photo, wrote = wrote, reminder = reminder, steps = steps }
"#;

fn package(tmp: &Path, manifest: &str) -> std::path::PathBuf {
    let dir = tmp.join("dev/meals");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    std::fs::write(dir.join("log.luau"), LOG).unwrap();
    dir
}

/// The app with the meal plan compiled in (health data is the app's own
/// extensions' alone).
fn app(tmp: &Path, device: Option<&FakeDevice>, manifest: &str) -> (ActionList, Extensions) {
    app_as(tmp, device, manifest, true)
}

fn app_as(
    tmp: &Path,
    device: Option<&FakeDevice>,
    manifest: &str,
    builtin: bool,
) -> (ActionList, Extensions) {
    let list = ActionList::new();
    let mut host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_data_root(tmp.join("data"));
    if let Some(device) = device {
        host = host.with_device(device.clone());
    }
    let mut ext = Extensions::new(host, Installs::new(tmp.join("installs")));
    if builtin {
        ext = ext.with_builtin(Package::embedded(manifest, &[("log.luau", LOG)]).unwrap());
    }
    ext.install_contribution_actions().unwrap();
    if builtin {
        let _ = ext.sync();
    } else {
        let _ = ext.dev(&package(tmp, manifest));
    }
    assert!(
        ext.host().get("lab.meals").is_some(),
        "{:?}",
        ext.problems()
    );
    (list, ext)
}

fn call(ext: &Extensions, request: &str) -> Result<Value, HostError> {
    let request: HostRequest = serde_json::from_str(request).unwrap();
    block_on(ext.host().api("lab.meals").call(request))
}

#[test]
fn a_luau_extension_takes_a_photo_writes_calories_reminds_and_reads_steps() {
    let tmp = tempfile::tempdir().unwrap();
    let meal = tmp.path().join("captures/meal.jpg");
    std::fs::create_dir_all(meal.parent().unwrap()).unwrap();
    std::fs::write(&meal, "jpeg").unwrap();
    let device = FakeDevice::new().with_image(meal.display().to_string());
    let (list, ext) = app(tmp.path(), Some(&device), MANIFEST);
    // A snack it logged before, outside the span read below.
    call(
        &ext,
        r#"{"op":"health_write","type":"dietary_energy","value":10,"start_ms":1500}"#,
    )
    .unwrap();
    let out = block_on(list.run_json_async_with(&ActionCaller::app(), "LAB_MEALS_LOG", b"{}"))
        .map(|o| serde_json::from_slice::<Value>(&o).unwrap())
        .unwrap();
    assert_eq!(out["photo"], json!({ "ok": meal.display().to_string() }));
    assert_eq!(out["reminder"], json!({ "ok": { "id": "dinner" } }));
    assert_eq!(
        out["steps"],
        json!({ "ok": { "type": "steps", "unit": "count", "sum": 0.0 } })
    );
    // Its reminder is under its own name: no other extension's "dinner".
    let scheduled = device.scheduled();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].0, "lab.meals/dinner");
    assert_eq!(scheduled[0].1.after_s, Some(3600));
    // The photo taken is one it may read; not write.
    assert_eq!(
        call(&ext, &json!({"op": "read_file", "path": meal}).to_string()),
        Ok(json!("jpeg"))
    );
    assert!(matches!(
        call(
            &ext,
            &json!({"op": "write_file", "path": meal, "text": "x"}).to_string()
        ),
        Err(HostError::Refused { .. })
    ));
    // Cancelled once, then it is gone.
    assert_eq!(
        call(&ext, r#"{"op":"cancel_os_notify","id":"dinner"}"#),
        Ok(json!(true))
    );
    assert_eq!(
        call(&ext, r#"{"op":"cancel_os_notify","id":"dinner"}"#),
        Ok(json!(false))
    );
    assert!(device.asked().contains(&DeviceCall::Capture));
    // A banner says which extension sent it (its subtitle).
    assert!(
        device
            .asked()
            .iter()
            .any(|c| matches!(c, DeviceCall::Notify(k, from, _) if k == "lab.meals/dinner" && from == "Meals")),
        "{:?}",
        device.asked()
    );
}

#[test]
fn an_extensions_notifications_go_when_it_unloads_or_loses_them() {
    let tmp = tempfile::tempdir().unwrap();
    let device = FakeDevice::new();
    let (_list, ext) = app(tmp.path(), Some(&device), MANIFEST);
    call(
        &ext,
        r#"{"op":"os_notify","id":"lunch","title":"Lunch","after_s":600}"#,
    )
    .unwrap();
    call(
        &ext,
        r#"{"op":"os_notify","id":"dinner","title":"Dinner","after_s":6000}"#,
    )
    .unwrap();
    assert_eq!(device.scheduled().len(), 2);
    // Unloaded (disabled, removed): none of its notifications is left.
    assert!(ext.host().unload("lab.meals"));
    assert!(device.scheduled().is_empty(), "{:?}", device.scheduled());
    assert!(
        device
            .asked()
            .contains(&DeviceCall::Forget("lab.meals".to_owned()))
    );
    // A grant taken back from one that may no longer notify: the same.
    ext.host().grants_changed("gone.ext");
    assert!(
        device
            .asked()
            .contains(&DeviceCall::Forget("gone.ext".to_owned()))
    );
}

#[test]
fn health_is_never_relayed_to_an_outside_caller_or_another_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let device = FakeDevice::new();
    let (_list, ext) = app(tmp.path(), Some(&device), MANIFEST);
    let read: HostRequest =
        serde_json::from_str(r#"{"op":"health_read","type":"steps","from_ms":0,"to_ms":1}"#)
            .unwrap();
    let as_for = |who: Option<Principal>| {
        block_on(
            ext.host()
                .api("lab.meals")
                .acting_for(who)
                .call(read.clone()),
        )
    };
    // For the person using the app (none), or its own run: read.
    assert!(as_for(None).is_ok());
    assert!(as_for(Some(Principal::extension("lab.meals", None))).is_ok());
    // For an agent over MCP, or a peer: refused.
    for who in [
        Principal::Service {
            name: "mcp".to_owned(),
        },
        Principal::extension(
            "lab.meals",
            Some(Principal::Service {
                name: "mcp".to_owned(),
            }),
        ),
    ] {
        match as_for(Some(who.clone())) {
            Err(HostError::Refused { capability, why }) => {
                assert_eq!(capability, "device.health");
                assert!(why.contains("outside caller"), "{why}");
            }
            other => panic!("{who:?}: {other:?}"),
        }
    }
    // A service call from an extension that is not built in: refused.
    assert!(matches!(
        as_for(Some(Principal::extension("someone.else", None))),
        Err(HostError::Refused { .. })
    ));
}

#[test]
fn a_development_copy_of_a_built_ins_id_gets_no_health_data() {
    let tmp = tempfile::tempdir().unwrap();
    let device = FakeDevice::new();
    // The meal plan compiled in, then a development copy of the same id
    // (it overrides the built-in, as EXT_CLONE's does).
    let (_list, ext) = app(tmp.path(), Some(&device), MANIFEST);
    let _ = ext.dev(&package(tmp.path(), MANIFEST));
    match call(
        &ext,
        r#"{"op":"health_read","type":"steps","from_ms":0,"to_ms":1}"#,
    ) {
        Err(HostError::Refused { capability, why }) => {
            assert_eq!(capability, "device.health");
            assert!(why.contains("compiled-in"), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn photos_are_listed_newest_first_and_copied_by_id() {
    let tmp = tempfile::tempdir().unwrap();
    let copy = tmp.path().join("picks/1.jpg");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::write(&copy, "jpeg").unwrap();
    let photo = |id: &str, at| PhotoAsset {
        id: id.to_owned(),
        created_ms: Some(at),
        width: 4032,
        height: 3024,
    };
    let device = FakeDevice::new()
        .with_image(copy.display().to_string())
        .with_photos(vec![photo("b", 2000), photo("a", 1000)]);
    let (_list, ext) = app(tmp.path(), Some(&device), MANIFEST);
    let listed = call(&ext, r#"{"op":"photos","since_ms":1500}"#).unwrap();
    assert_eq!(
        listed,
        json!([{ "id": "b", "created_ms": 2000, "width": 4032, "height": 3024 }])
    );
    assert_eq!(
        call(&ext, r#"{"op":"photo_file","id":"b"}"#),
        Ok(json!(copy.display().to_string()))
    );
    assert_eq!(
        call(&ext, r#"{"op":"photo_file","id":"zz"}"#),
        Ok(Value::Null)
    );
    assert_eq!(
        call(&ext, &json!({"op": "read_file", "path": copy}).to_string()),
        Ok(json!("jpeg"))
    );
}

#[test]
fn every_device_call_is_refused_outside_its_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let device = FakeDevice::new();
    // It asks to read steps, and nothing else.
    let manifest = MANIFEST.replace(
        r#"device = ["notifications", "camera", "photos", "health:read:steps", "health:write:dietary_energy"]"#,
        r#"device = ["health:read:steps"]"#,
    );
    let (_list, ext) = app(tmp.path(), Some(&device), &manifest);
    let calls = [
        (r#"{"op":"os_notify","title":"t"}"#, "device.notifications"),
        (
            r#"{"op":"cancel_os_notify","id":"x"}"#,
            "device.notifications",
        ),
        (r#"{"op":"capture"}"#, "device.camera"),
        (r#"{"op":"photos"}"#, "device.photos"),
        (r#"{"op":"photo_file","id":"x"}"#, "device.photos"),
        // Writing a type it may read is another entry.
        (
            r#"{"op":"health_write","type":"steps","value":1}"#,
            "device.health:write:steps",
        ),
        (
            r#"{"op":"health_read","type":"body_mass","from_ms":0,"to_ms":1}"#,
            "device.health:read:body_mass",
        ),
        (
            r#"{"op":"health_read","type":"blood","from_ms":0,"to_ms":1}"#,
            "device",
        ),
    ];
    for (request, needs) in calls {
        match call(&ext, request) {
            Err(HostError::Refused { capability, .. }) => {
                assert_eq!(capability, needs, "{request}");
            }
            other => panic!("{request}: {other:?}"),
        }
    }
    assert!(device.asked().is_empty(), "nothing reached the device");
    // The one it asked for works.
    assert!(
        call(
            &ext,
            r#"{"op":"health_read","type":"steps","from_ms":0,"to_ms":1}"#
        )
        .is_ok()
    );
}

#[test]
fn health_data_is_the_compiled_in_extensions_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let device = FakeDevice::new();
    // The same manifest, as a development copy (an installed one is the
    // same): the entry is there, the data is not.
    let (_list, ext) = app_as(tmp.path(), Some(&device), MANIFEST, false);
    for request in [
        r#"{"op":"health_read","type":"steps","from_ms":0,"to_ms":1}"#,
        r#"{"op":"health_write","type":"dietary_energy","value":1}"#,
    ] {
        match call(&ext, request) {
            Err(HostError::Refused { capability, why }) => {
                assert_eq!(capability, "device.health", "{request}");
                assert!(why.contains("compiled-in"), "{why}");
            }
            other => panic!("{request}: {other:?}"),
        }
    }
    assert!(device.asked().is_empty(), "nothing reached the device");
    // Its other device entries still work.
    assert!(call(&ext, r#"{"op":"os_notify","title":"t"}"#).is_ok());
}

#[test]
fn a_host_without_a_device_says_it_cannot() {
    let tmp = tempfile::tempdir().unwrap();
    let (_list, ext) = app(tmp.path(), None, MANIFEST);
    for request in [
        r#"{"op":"os_notify","title":"t"}"#,
        r#"{"op":"capture"}"#,
        r#"{"op":"health_read","type":"steps","from_ms":0,"to_ms":1}"#,
    ] {
        assert!(
            matches!(call(&ext, request), Err(HostError::Unsupported { .. })),
            "{request}"
        );
    }
}

#[test]
fn a_manifest_names_only_known_entries_and_consent_lists_each() {
    let bad = |device: &str| {
        Manifest::parse(&MANIFEST.replace(
            r#"device = ["notifications", "camera", "photos", "health:read:steps", "health:write:dietary_energy"]"#,
            &format!("device = [{device:?}]"),
        ))
        .is_err()
    };
    assert!(bad("location"));
    assert!(bad("health:read:*"));
    assert!(bad("health:read:blood"));
    assert!(!bad("health:write:water"));
    let manifest = Manifest::parse(MANIFEST).unwrap();
    let asks = manifest.capabilities.asks();
    let says: Vec<&str> = asks
        .iter()
        .filter(|a| a.kind == "device")
        .map(|a| a.says.as_str())
        .collect();
    assert_eq!(
        says,
        [
            "Send you notifications, now or at a time it sets",
            "Ask you to take a photo with the camera",
            "See your photo library",
            "Read your steps from Health",
            "Add energy eaten (calories) to Health",
        ]
    );
    // Granted the camera and steps only: that is all it may use.
    let granted = Capabilities {
        device: vec!["camera".to_owned(), "health:read:steps".to_owned()],
        ..Capabilities::default()
    };
    let may = manifest.capabilities.within(&granted);
    assert_eq!(may.device, ["camera", "health:read:steps"]);
    assert_eq!(manifest.capabilities.grows_from(&granted).len(), 3);
}
