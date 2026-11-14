//! The device's own services for extensions (#709).
//!
//! OS notifications, the camera, the photo library and health data, each
//! a host call gated by an entry of `capabilities.device` and answered by
//! the platform's [`Device`].
//!
//! | call | what it does | answer | `capabilities.device` |
//! |---|---|---|---|
//! | `os_notify {id?, title, body?, at_ms?, after_s?}` | an OS notification: now, at a time (ms since the epoch) or after some seconds; one with the same `id` replaces it | `{id}` | `notifications` |
//! | `cancel_os_notify {id}` | takes one it scheduled back | whether it had one | `notifications` |
//! | `capture {kind?}` | the camera: the person takes a photo | the path of the image, `null` when cancelled | `camera` |
//! | `photos {limit?, since_ms?}` | the newest photos in the library | `[{id, created_ms, width, height}]` | `photos` |
//! | `photo_file {id}` | one of them as a file copied for it | its path | `photos` |
//! | `health_read {type, from_ms, to_ms, sum?}` | health data of one [`HealthType`] over a span | `{type, unit, sum}` or `{type, unit, samples: [{value, start_ms, end_ms}]}` | `health:read:<type>` |
//! | `health_write {type, value, start_ms?, end_ms?}` | one sample of a [`HealthType`] | `null` | `health:write:<type>` |
//!
//! The shape is the platform's business only inside [`Device`]: iOS
//! answers with `UNUserNotificationCenter`, `UIImagePickerController`,
//! `Photos` and `HealthKit` (`crate::device_ios`); a desk or a browser
//! answers [`HostError::Unsupported`] until it has its own. The OS asks
//! the person too (its permission prompts), the first time an extension
//! needs each; a capability not in the manifest (or not granted) is
//! refused before the OS is asked.
//!
//! Health data is an allow-list: each type, read and write apart
//! (`health:read:steps`, `health:write:dietary_energy`), named in the
//! manifest and shown in the consent prompt. Only [`HEALTH_TYPES`] exist.
//!
//! A file it is answered (a capture, a photo's copy) is one it may read
//! for the run, as a file the person picked.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hostcall::HostError;

/// `capabilities.device`: OS notifications.
pub const NOTIFICATIONS: &str = "notifications";
/// `capabilities.device`: the camera.
pub const CAMERA: &str = "camera";
/// `capabilities.device`: the photo library (read).
pub const PHOTOS: &str = "photos";
/// The plain entries of `capabilities.device`; health's are
/// `health:read:<type>` and `health:write:<type>`.
pub const DEVICE: [&str; 3] = [NOTIFICATIONS, CAMERA, PHOTOS];

/// How a health type's values add up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    /// Summed over a span (steps, energy, water).
    Sum,
    /// Each a reading (weight, heart rate): read as samples.
    Discrete,
}

/// One kind of health data an extension may name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct HealthType {
    /// Its name in a manifest and a call (`steps`).
    pub name: &'static str,
    /// What a person reads in a consent prompt.
    pub title: &'static str,
    /// `HealthKit`'s quantity type identifier.
    pub healthkit: &'static str,
    /// The unit values are in (`HKUnit`'s string).
    pub unit: &'static str,
    pub aggregate: Aggregate,
}

const fn ht(
    name: &'static str,
    title: &'static str,
    healthkit: &'static str,
    unit: &'static str,
    aggregate: Aggregate,
) -> HealthType {
    HealthType {
        name,
        title,
        healthkit,
        unit,
        aggregate,
    }
}

/// The health data an extension may ask for.
pub const HEALTH_TYPES: [HealthType; 10] = [
    ht(
        "steps",
        "steps",
        "HKQuantityTypeIdentifierStepCount",
        "count",
        Aggregate::Sum,
    ),
    ht(
        "active_energy",
        "active energy burned",
        "HKQuantityTypeIdentifierActiveEnergyBurned",
        "kcal",
        Aggregate::Sum,
    ),
    ht(
        "dietary_energy",
        "energy eaten (calories)",
        "HKQuantityTypeIdentifierDietaryEnergyConsumed",
        "kcal",
        Aggregate::Sum,
    ),
    ht(
        "protein",
        "protein eaten",
        "HKQuantityTypeIdentifierDietaryProtein",
        "g",
        Aggregate::Sum,
    ),
    ht(
        "carbohydrates",
        "carbohydrates eaten",
        "HKQuantityTypeIdentifierDietaryCarbohydrates",
        "g",
        Aggregate::Sum,
    ),
    ht(
        "fat",
        "fat eaten",
        "HKQuantityTypeIdentifierDietaryFatTotal",
        "g",
        Aggregate::Sum,
    ),
    ht(
        "water",
        "water drunk",
        "HKQuantityTypeIdentifierDietaryWater",
        "mL",
        Aggregate::Sum,
    ),
    ht(
        "distance",
        "walking and running distance",
        "HKQuantityTypeIdentifierDistanceWalkingRunning",
        "m",
        Aggregate::Sum,
    ),
    ht(
        "body_mass",
        "weight",
        "HKQuantityTypeIdentifierBodyMass",
        "kg",
        Aggregate::Discrete,
    ),
    ht(
        "heart_rate",
        "heart rate",
        "HKQuantityTypeIdentifierHeartRate",
        "count/min",
        Aggregate::Discrete,
    ),
];

/// The health type named `name`.
#[must_use]
pub fn health_type(name: &str) -> Option<&'static HealthType> {
    HEALTH_TYPES.iter().find(|t| t.name == name)
}

/// Whether a person may read (`write` false) or write health type `name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthAccess {
    Read,
    Write,
}

impl HealthAccess {
    const fn word(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// `capabilities.device`'s entry for `access` to health type `name`.
#[must_use]
pub fn health_entry(access: HealthAccess, name: &str) -> String {
    format!("health:{}:{name}", access.word())
}

/// Whether `entry` is a well-formed `capabilities.device` entry: why not.
///
/// # Errors
///
/// What is wrong with it.
pub fn check_entry(entry: &str) -> Result<(), String> {
    if DEVICE.contains(&entry) {
        return Ok(());
    }
    let names = || {
        HEALTH_TYPES
            .iter()
            .map(|t| t.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match entry.split(':').collect::<Vec<_>>().as_slice() {
        ["health", "read" | "write", name] if health_type(name).is_some() => Ok(()),
        ["health", "read" | "write", name] => {
            Err(format!("no health type {name:?}: one of {}", names()))
        }
        _ => Err(format!(
            "one of {}, health:read:<type> or health:write:<type> (types: {})",
            DEVICE.join(", "),
            names()
        )),
    }
}

/// The sentence a consent prompt shows for `entry`.
#[must_use]
pub fn words(entry: &str) -> String {
    match entry {
        NOTIFICATIONS => "Send you notifications, now or at a time it sets".to_owned(),
        CAMERA => "Ask you to take a photo with the camera".to_owned(),
        PHOTOS => "See your photo library".to_owned(),
        _ => match entry.split(':').collect::<Vec<_>>().as_slice() {
            ["health", "read", name] => format!(
                "Read your {} from Health",
                health_type(name).map_or(*name, |t| t.title)
            ),
            ["health", "write", name] => format!(
                "Add {} to Health",
                health_type(name).map_or(*name, |t| t.title)
            ),
            _ => format!("Use {entry}"),
        },
    }
}

/// An OS notification (`os_notify`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OsNotification {
    /// Its id, to replace or cancel it; one is made when none.
    pub id: Option<String>,
    pub title: String,
    pub body: Option<String>,
    /// When, in ms since the epoch (none, or past: now).
    pub at_ms: Option<u64>,
    /// After this many seconds (`at_ms` wins).
    pub after_s: Option<u64>,
}

impl OsNotification {
    /// How many seconds from `now_ms` it is due; `None`: now.
    #[must_use]
    pub fn delay_s(&self, now_ms: u64) -> Option<f64> {
        let ms = match (self.at_ms, self.after_s) {
            (Some(at), _) => at.checked_sub(now_ms)?,
            (None, Some(s)) => s.saturating_mul(1000),
            (None, None) => return None,
        };
        let s = std::time::Duration::from_millis(ms).as_secs_f64();
        (s >= 1.0).then_some(s)
    }
}

/// What the camera takes (`capture`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    #[default]
    Photo,
}

/// A camera capture (`capture`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capture {
    pub kind: CaptureKind,
}

/// Which photos (`photos`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoQuery {
    /// At most this many, newest first (20 when none, 200 at most).
    pub limit: Option<u32>,
    /// Taken at or after, in ms since the epoch.
    pub since_ms: Option<u64>,
}

impl PhotoQuery {
    /// How many to answer.
    #[must_use]
    pub fn limit(&self) -> u32 {
        self.limit.unwrap_or(20).clamp(1, 200)
    }
}

/// One photo in the library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoAsset {
    /// The library's id for it (`photo_file` takes it).
    pub id: String,
    pub created_ms: Option<u64>,
    pub width: u32,
    pub height: u32,
}

/// Health data over a span (`health_read`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthQuery {
    /// A [`HealthType`]'s name.
    #[serde(rename = "type")]
    pub kind: String,
    pub from_ms: u64,
    pub to_ms: u64,
    /// The sum over the span (a [`Aggregate::Sum`] type's default), not
    /// the samples.
    #[serde(default)]
    pub sum: Option<bool>,
}

impl HealthQuery {
    /// Whether it asks for the sum.
    #[must_use]
    pub fn wants_sum(&self, of: &HealthType) -> bool {
        self.sum.unwrap_or(of.aggregate == Aggregate::Sum) && of.aggregate == Aggregate::Sum
    }
}

/// One sample, read or written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HealthSample {
    pub value: f64,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// One sample to write (`health_write`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HealthWrite {
    #[serde(rename = "type")]
    pub kind: String,
    /// In the type's unit ([`HealthType::unit`]).
    pub value: f64,
    /// When it began (none: now), in ms since the epoch.
    #[serde(default)]
    pub start_ms: Option<u64>,
    /// When it ended (none: when it began).
    #[serde(default)]
    pub end_ms: Option<u64>,
}

// A request is compared whole (`HostRequest: Eq`); a value is a finite
// number by the time it is written (the host refuses NaN and infinities).
impl Eq for HealthWrite {}

/// What health data a read answers.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum HealthAnswer {
    Sum {
        #[serde(rename = "type")]
        kind: String,
        unit: String,
        sum: f64,
    },
    Samples {
        #[serde(rename = "type")]
        kind: String,
        unit: String,
        samples: Vec<HealthSample>,
    },
}

#[cfg(feature = "host")]
pub use service::{Device, DeviceCall, FakeDevice, NoDevice};

/// The platform's side: [`Device`] and the fakes (feature `host`).
#[cfg(feature = "host")]
mod service {
    use super::{
        Capture, HealthAnswer, HealthQuery, HealthSample, HealthType, OsNotification, PhotoAsset,
        PhotoQuery,
    };
    use crate::hostui::{Asked, unsupported};

    /// The device's services, as a platform has them. Every one is
    /// [`HostError::Unsupported`] unless the platform answers it.
    ///
    /// A notification's `key` is the extension's id and its own id
    /// (`<ext>/<id>`), so one extension never replaces another's.
    pub trait Device: Send + Sync {
        /// Shows or schedules `notification` under `key`.
        fn notify(&self, key: &str, notification: &OsNotification) -> Asked<()> {
            let _ = (key, notification);
            no("OS notifications")
        }

        /// Cancels `key`'s notification: whether there was one.
        fn cancel_notification(&self, key: &str) -> Asked<bool> {
            let _ = key;
            no("OS notifications")
        }

        /// The camera: the image's path, `None` when the person cancelled.
        fn capture(&self, capture: &Capture) -> Asked<Option<String>> {
            let _ = capture;
            no("the camera")
        }

        /// The newest photos.
        fn photos(&self, query: &PhotoQuery) -> Asked<Vec<PhotoAsset>> {
            let _ = query;
            no("the photo library")
        }

        /// Photo `id` as a file copied for the extension.
        fn photo_file(&self, id: &str) -> Asked<Option<String>> {
            let _ = id;
            no("the photo library")
        }

        /// Health data of `of` over the query's span.
        fn health_read(&self, of: &'static HealthType, query: &HealthQuery) -> Asked<HealthAnswer> {
            let _ = (of, query);
            no("health data")
        }

        /// Writes one sample of `of`.
        fn health_write(&self, of: &'static HealthType, sample: &HealthSample) -> Asked<()> {
            let _ = (of, sample);
            no("health data")
        }
    }

    fn no<T: Send + 'static>(what: &str) -> Asked<T> {
        let e = unsupported(&format!("{what} on this platform"));
        Box::pin(async move { Err(e) })
    }

    /// A platform without device services: every call unsupported.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct NoDevice;

    impl Device for NoDevice {}

    /// What a [`FakeDevice`] was asked.
    #[derive(Clone, Debug, PartialEq)]
    pub enum DeviceCall {
        Notify(String, OsNotification),
        Cancel(String),
        Capture,
        Photos(PhotoQuery),
        PhotoFile(String),
        HealthRead(String, HealthQuery),
        HealthWrite(String, HealthSample),
    }

    /// A device for tests and headless hosts: notifications kept, a capture
    /// and a photo answered with the path it was given, health data from
    /// what was written to it.
    #[derive(Clone, Debug, Default)]
    pub struct FakeDevice {
        state: std::sync::Arc<std::sync::Mutex<FakeState>>,
    }

    #[derive(Debug, Default)]
    struct FakeState {
        asked: Vec<DeviceCall>,
        scheduled: std::collections::BTreeMap<String, OsNotification>,
        image: Option<String>,
        photos: Vec<PhotoAsset>,
        health: Vec<(String, HealthSample)>,
    }

    impl FakeDevice {
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }

        fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        /// The image a capture and `photo_file` answer.
        #[must_use]
        pub fn with_image(self, path: impl Into<String>) -> Self {
            self.state().image = Some(path.into());
            self
        }

        /// The library's photos.
        #[must_use]
        pub fn with_photos(self, photos: Vec<PhotoAsset>) -> Self {
            self.state().photos = photos;
            self
        }

        /// Everything it was asked, in order.
        #[must_use]
        pub fn asked(&self) -> Vec<DeviceCall> {
            self.state().asked.clone()
        }

        /// The notifications scheduled now, by key.
        #[must_use]
        pub fn scheduled(&self) -> Vec<(String, OsNotification)> {
            self.state()
                .scheduled
                .iter()
                .map(|(k, n)| (k.clone(), n.clone()))
                .collect()
        }
    }

    fn ready<T: Send + 'static>(value: T) -> Asked<T> {
        Box::pin(async move { Ok(value) })
    }

    impl Device for FakeDevice {
        fn notify(&self, key: &str, notification: &OsNotification) -> Asked<()> {
            {
                let mut s = self.state();
                s.asked
                    .push(DeviceCall::Notify(key.to_owned(), notification.clone()));
                s.scheduled.insert(key.to_owned(), notification.clone());
            }
            ready(())
        }

        fn cancel_notification(&self, key: &str) -> Asked<bool> {
            let mut s = self.state();
            s.asked.push(DeviceCall::Cancel(key.to_owned()));
            ready(s.scheduled.remove(key).is_some())
        }

        fn capture(&self, _capture: &Capture) -> Asked<Option<String>> {
            let mut s = self.state();
            s.asked.push(DeviceCall::Capture);
            ready(s.image.clone())
        }

        fn photos(&self, query: &PhotoQuery) -> Asked<Vec<PhotoAsset>> {
            let limit = usize::try_from(query.limit()).unwrap_or(usize::MAX);
            let mut s = self.state();
            s.asked.push(DeviceCall::Photos(query.clone()));
            let photos: Vec<PhotoAsset> = s
                .photos
                .iter()
                .filter(|p| {
                    query
                        .since_ms
                        .is_none_or(|t| p.created_ms.is_some_and(|c| c >= t))
                })
                .take(limit)
                .cloned()
                .collect();
            drop(s);
            ready(photos)
        }

        fn photo_file(&self, id: &str) -> Asked<Option<String>> {
            let mut s = self.state();
            s.asked.push(DeviceCall::PhotoFile(id.to_owned()));
            let known = s.photos.iter().any(|p| p.id == id);
            ready(s.image.clone().filter(|_| known))
        }

        fn health_read(&self, of: &'static HealthType, query: &HealthQuery) -> Asked<HealthAnswer> {
            let mut s = self.state();
            s.asked
                .push(DeviceCall::HealthRead(of.name.to_owned(), query.clone()));
            let samples: Vec<HealthSample> = s
                .health
                .iter()
                .filter(|(k, h)| {
                    k == of.name && h.start_ms >= query.from_ms && h.end_ms <= query.to_ms
                })
                .map(|(_, h)| h.clone())
                .collect();
            drop(s);
            let (kind, unit) = (of.name.to_owned(), of.unit.to_owned());
            ready(if query.wants_sum(of) {
                HealthAnswer::Sum {
                    kind,
                    unit,
                    sum: samples.iter().map(|h| h.value).sum(),
                }
            } else {
                HealthAnswer::Samples {
                    kind,
                    unit,
                    samples,
                }
            })
        }

        fn health_write(&self, of: &'static HealthType, sample: &HealthSample) -> Asked<()> {
            {
                let mut s = self.state();
                s.asked
                    .push(DeviceCall::HealthWrite(of.name.to_owned(), sample.clone()));
                s.health.push((of.name.to_owned(), sample.clone()));
            }
            ready(())
        }
    }
}

/// A health read's answer as the call gives it.
#[must_use]
pub fn health_answer(answer: &HealthAnswer) -> Value {
    serde_json::to_value(answer).unwrap_or(Value::Null)
}

/// Why a call failed, when a platform's service did.
#[must_use]
pub fn failed(message: impl Into<String>) -> HostError {
    HostError::Failed {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_the_plain_ones_and_health_types_by_name() {
        assert_eq!(check_entry("notifications"), Ok(()));
        assert_eq!(check_entry("health:read:steps"), Ok(()));
        assert_eq!(check_entry("health:write:dietary_energy"), Ok(()));
        assert!(check_entry("health:read:*").is_err());
        assert!(
            check_entry("health:read:blood")
                .unwrap_err()
                .contains("steps")
        );
        assert!(check_entry("location").is_err());
        assert_eq!(words("health:read:steps"), "Read your steps from Health");
        assert_eq!(words("health:write:water"), "Add water drunk to Health");
    }

    #[test]
    fn a_notification_is_due_now_at_a_time_or_after_a_while() {
        let n = |at_ms, after_s| OsNotification {
            title: "t".to_owned(),
            at_ms,
            after_s,
            ..OsNotification::default()
        };
        assert_eq!(n(None, None).delay_s(1_000), None);
        assert_eq!(n(Some(500), None).delay_s(1_000), None, "past: now");
        assert_eq!(n(Some(61_000), Some(5)).delay_s(1_000), Some(60.0));
        assert_eq!(n(None, Some(90)).delay_s(1_000), Some(90.0));
    }

    #[test]
    fn a_sum_is_only_for_a_summed_type() {
        let q = |sum| HealthQuery {
            kind: "x".to_owned(),
            from_ms: 0,
            to_ms: 1,
            sum,
        };
        let steps = health_type("steps").unwrap();
        let weight = health_type("body_mass").unwrap();
        assert!(q(None).wants_sum(steps));
        assert!(!q(Some(false)).wants_sum(steps));
        assert!(!q(None).wants_sum(weight));
        assert!(!q(Some(true)).wants_sum(weight));
    }
}
