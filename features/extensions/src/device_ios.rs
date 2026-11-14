//! The device's services on iOS (#709): [`IosDevice`], the platform's
//! [`Device`].
//!
//! - **Notifications**: `UNUserNotificationCenter`. Authorization is asked
//!   the first time an extension sends one (the OS's prompt); a delivery
//!   while the app is in front shows as a banner (the center's delegate).
//!   Each request's identifier is `<extension>/<its id>`.
//! - **The camera**: `UIImagePickerController` with the camera as its
//!   source, presented over the key window; the photo is written as a
//!   JPEG to `<tmp>/architect-captures/`. A device with no camera (the
//!   Simulator) answers unsupported.
//! - **The photo library**: `PHAsset` (newest first) after the OS's
//!   read-write authorization (limited access counts); a photo's data from
//!   `PHImageManager`, copied to `<tmp>/architect-picks/`.
//! - **Health**: `HKHealthStore`, authorization asked for the one type
//!   and direction the call needs. A sum is an `HKStatisticsQuery`'s
//!   cumulative sum; samples an `HKSampleQuery`'s, oldest first; a write
//!   one `HKQuantitySample`. The app needs the `HealthKit` entitlement and
//!   both health usage strings in its Info.plist
//!   (`apps/mobile/ios/Info.plist`, `apps/mobile/ios/Architect.entitlements`).
//!   `HealthKit` never says whether reading was refused: a refused read
//!   answers no data.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use block2::RcBlock;
use futures::channel::oneshot;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_foundation::{
    NSArray, NSCopying, NSData, NSDate, NSDictionary, NSError, NSSet, NSSortDescriptor, NSString,
};
use objc2_health_kit::{
    HKHealthStore, HKObjectType, HKQuantity, HKQuantitySample, HKQuantityType, HKQuery,
    HKQueryOptions, HKSample, HKSampleQuery, HKSampleType, HKStatistics, HKStatisticsOptions,
    HKStatisticsQuery, HKUnit,
};
use objc2_photos::{
    PHAccessLevel, PHAsset, PHAssetMediaType, PHAuthorizationStatus, PHFetchOptions,
    PHImageManager, PHImageRequestOptions, PHImageRequestOptionsDeliveryMode, PHPhotoLibrary,
};
use objc2_ui_kit::{
    UIApplication, UIImage, UIImagePickerController, UIImagePickerControllerOriginalImage,
    UIImagePickerControllerSourceType, UIViewController, UIWindowScene,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationSound,
    UNNotificationTrigger, UNTimeIntervalNotificationTrigger, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

use crate::device::{
    Aggregate, Capture, Device, HealthAnswer, HealthQuery, HealthSample, HealthType,
    OsNotification, PhotoAsset, PhotoQuery, failed,
};
use crate::hostcall::HostError;
use crate::hostui::{Asked, unsupported};

/// The device's services on an iPhone or iPad.
#[derive(Clone, Copy, Debug, Default)]
pub struct IosDevice;

impl IosDevice {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

type Answer<T> = Result<T, HostError>;

/// A one-shot reply a block (an `Fn`) can send once.
struct Once<T>(Mutex<Option<oneshot::Sender<Answer<T>>>>);

impl<T> Once<T> {
    fn new(tx: oneshot::Sender<Answer<T>>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Some(tx))))
    }

    fn send(&self, answer: Answer<T>) {
        let tx = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(tx) = tx {
            let _ = tx.send(answer);
        }
    }
}

/// `start` given a reply to send once; the call's future waits for it.
fn asked<T: Send + 'static>(start: impl FnOnce(Arc<Once<T>>)) -> Asked<T> {
    let (tx, rx) = oneshot::channel();
    start(Once::new(tx));
    Box::pin(async move {
        rx.await
            .unwrap_or_else(|_| Err(failed("the device's answer never came")))
    })
}

/// `work` on the main thread.
fn on_main(work: impl FnOnce(MainThreadMarker) + Send + 'static) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        if let Some(mtm) = MainThreadMarker::new() {
            work(mtm);
        }
    });
}

fn error_text(error: *mut NSError) -> String {
    // SAFETY: a completion handler's error is nil or a live NSError.
    unsafe { error.as_ref() }.map_or_else(
        || "the device refused".to_owned(),
        |e| e.localizedDescription().to_string(),
    )
}

fn date(ms: u64) -> Retained<NSDate> {
    NSDate::dateWithTimeIntervalSince1970(std::time::Duration::from_millis(ms).as_secs_f64())
}

fn ms(date: &NSDate) -> u64 {
    std::time::Duration::try_from_secs_f64(date.timeIntervalSince1970().max(0.0))
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn tmp_file(folder: &str, ext: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(folder);
    let _ = std::fs::create_dir_all(&dir);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    dir.join(format!("{now}-{}.{ext}", std::process::id()))
}

// --- Notifications -----------------------------------------------------------

define_class!(
    // SAFETY: an NSObject subclass with no ivars and nothing of NSObject's
    // overridden; the center calls it on a queue of its own.
    #[unsafe(super(NSObject))]
    #[name = "ArchitectExtensionNotifications"]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        /// One that arrives while the app is in front shows anyway.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }
    }
);

/// The center, its delegate set once (and kept for the run).
fn center() -> Retained<UNUserNotificationCenter> {
    static DELEGATE: std::sync::Once = std::sync::Once::new();
    let center = UNUserNotificationCenter::currentNotificationCenter();
    DELEGATE.call_once(|| {
        // SAFETY: NSObject's `init` on a fresh instance.
        let delegate: Retained<NotificationDelegate> =
            unsafe { msg_send![NotificationDelegate::alloc(), init] };
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The center holds its delegate weakly: this one lives for the run.
        std::mem::forget(delegate);
    });
    center
}

/// Asks for the OS's permission (the prompt, the first time), then `then`.
fn authorized(center: &UNUserNotificationCenter, reply: Arc<Once<()>>, then: impl Fn() + 'static) {
    let block = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        if granted.as_bool() {
            then();
        } else if error.is_null() {
            reply.send(Err(failed(
                "notifications are off for this app (Settings → Notifications)",
            )));
        } else {
            reply.send(Err(failed(error_text(error))));
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert
            | UNAuthorizationOptions::Sound
            | UNAuthorizationOptions::Badge,
        &block,
    );
}

// --- The camera -------------------------------------------------------------

struct CameraIvars {
    reply: RefCell<Option<Arc<Once<Option<String>>>>>,
}

define_class!(
    // SAFETY: an NSObject subclass with no superclass method overridden;
    // UIKit calls it on the main thread. It answers
    // UIImagePickerControllerDelegate's two methods, by selector.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ArchitectExtensionCamera"]
    #[ivars = CameraIvars]
    struct CameraDelegate;

    unsafe impl NSObjectProtocol for CameraDelegate {}

    impl CameraDelegate {
        #[unsafe(method(imagePickerController:didFinishPickingMediaWithInfo:))]
        fn did_finish(&self, picker: &UIImagePickerController, info: &NSDictionary<NSString, AnyObject>) {
            picker.dismissViewControllerAnimated_completion(true, None);
            // SAFETY: UIKit's info key.
            let key = unsafe { UIImagePickerControllerOriginalImage };
            let image = info
                .objectForKey(key)
                .and_then(|o| o.downcast::<UIImage>().ok());
            let saved = image.and_then(|image| {
                let data = UIImage::jpeg_representation(&image, 0.9)?;
                let path = tmp_file("architect-captures", "jpg");
                std::fs::write(&path, data.to_vec()).ok()?;
                Some(path.display().to_string())
            });
            self.answer(saved.map_or_else(
                || Err(failed("the photo taken could not be saved")),
                |p| Ok(Some(p)),
            ));
        }

        #[unsafe(method(imagePickerControllerDidCancel:))]
        fn did_cancel(&self, picker: &UIImagePickerController) {
            picker.dismissViewControllerAnimated_completion(true, None);
            self.answer(Ok(None));
        }
    }
);

thread_local! {
    /// The camera's delegate while it shows (the picker holds it weakly).
    static CAMERA: RefCell<Option<Retained<CameraDelegate>>> = const { RefCell::new(None) };
}

impl CameraDelegate {
    fn answer(&self, answer: Answer<Option<String>>) {
        if let Some(reply) = self.ivars().reply.borrow_mut().take() {
            reply.send(answer);
        }
        CAMERA.with(|c| c.borrow_mut().take());
    }
}

/// The view controller on top: the key window's root, then whatever it
/// presents.
fn top(mtm: MainThreadMarker) -> Option<Retained<UIViewController>> {
    let mut top = UIApplication::sharedApplication(mtm)
        .connectedScenes()
        .iter()
        .filter_map(|s| s.downcast::<UIWindowScene>().ok())
        .find_map(|s| s.keyWindow())
        .and_then(|w| w.rootViewController())?;
    while let Some(next) = top.presentedViewController() {
        top = next;
    }
    Some(top)
}

fn take_photo(mtm: MainThreadMarker, reply: Arc<Once<Option<String>>>) {
    let camera = UIImagePickerControllerSourceType::Camera;
    if !UIImagePickerController::isSourceTypeAvailable(camera, mtm) {
        reply.send(Err(unsupported("camera here (this device has none)")));
        return;
    }
    let Some(over) = top(mtm) else {
        reply.send(Err(failed("the app has no window to show the camera over")));
        return;
    };
    let picker = UIImagePickerController::new(mtm);
    picker.setSourceType(camera);
    let delegate = CameraDelegate::alloc(mtm).set_ivars(CameraIvars {
        reply: RefCell::new(Some(reply)),
    });
    // SAFETY: NSObject's `init`.
    let delegate: Retained<CameraDelegate> = unsafe { msg_send![super(delegate), init] };
    // SAFETY: the delegate answers the picker's delegate methods by
    // selector; the property takes any object.
    let any: &AnyObject = &delegate;
    unsafe { picker.setDelegate(Some(any)) };
    CAMERA.with(|c| *c.borrow_mut() = Some(delegate));
    over.presentViewController_animated_completion(&picker, true, None);
}

// --- Photos -----------------------------------------------------------------

/// The library, after the OS's permission: `then` with it allowed.
fn photos_allowed<T: Send + 'static>(reply: Arc<Once<T>>, then: impl Fn(Arc<Once<T>>) + 'static) {
    let block = RcBlock::new(move |status: PHAuthorizationStatus| {
        if status == PHAuthorizationStatus::Authorized || status == PHAuthorizationStatus::Limited {
            then(Arc::clone(&reply));
        } else {
            reply.send(Err(failed(
                "the photo library is off for this app (Settings → Privacy → Photos)",
            )));
        }
    });
    // SAFETY: Photos' own call; the handler runs once, on its queue.
    unsafe {
        PHPhotoLibrary::requestAuthorizationForAccessLevel_handler(
            PHAccessLevel::ReadWrite,
            &block,
        );
    };
}

fn newest(query: &PhotoQuery) -> Vec<PhotoAsset> {
    // SAFETY: Photos' fetch, read where it is made.
    unsafe {
        let options = PHFetchOptions::new();
        let by = NSSortDescriptor::sortDescriptorWithKey_ascending(
            Some(&NSString::from_str("creationDate")),
            false,
        );
        options.setSortDescriptors(Some(&NSArray::from_retained_slice(&[by])));
        options.setFetchLimit(usize::try_from(query.limit()).unwrap_or(20));
        let found =
            PHAsset::fetchAssetsWithMediaType_options(PHAssetMediaType::Image, Some(&options));
        (0..found.count())
            .map(|i| found.objectAtIndex(i))
            .map(|a| PhotoAsset {
                id: a.localIdentifier().to_string(),
                created_ms: a.creationDate().map(|d| ms(&d)),
                width: u32::try_from(a.pixelWidth()).unwrap_or(u32::MAX),
                height: u32::try_from(a.pixelHeight()).unwrap_or(u32::MAX),
            })
            .filter(|p| {
                query
                    .since_ms
                    .is_none_or(|t| p.created_ms.is_some_and(|c| c >= t))
            })
            .collect()
    }
}

fn photo_copy(id: &str, reply: Arc<Once<Option<String>>>) {
    // SAFETY: Photos' fetch and image manager; the handler runs once.
    unsafe {
        let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
        let found = PHAsset::fetchAssetsWithLocalIdentifiers_options(&ids, None);
        if found.count() == 0 {
            reply.send(Ok(None));
            return;
        }
        let asset = found.objectAtIndex(0);
        let options = PHImageRequestOptions::new();
        options.setNetworkAccessAllowed(true);
        options.setDeliveryMode(PHImageRequestOptionsDeliveryMode::HighQualityFormat);
        let block = RcBlock::new(
            move |data: *mut NSData, uti: *mut NSString, _orientation, _info: *mut NSDictionary| {
                let uti = uti.as_ref().map(ToString::to_string).unwrap_or_default();
                let ext = match uti.as_str() {
                    "public.heic" => "heic",
                    "public.png" => "png",
                    _ => "jpg",
                };
                let saved = data.as_ref().and_then(|d| {
                    let path = tmp_file("architect-picks", ext);
                    std::fs::write(&path, d.to_vec()).ok()?;
                    Some(path.display().to_string())
                });
                reply.send(saved.map_or_else(
                    || {
                        Err(failed(
                            "the photo could not be read (it may be in iCloud only)",
                        ))
                    },
                    |p| Ok(Some(p)),
                ));
            },
        );
        PHImageManager::defaultManager()
            .requestImageDataAndOrientationForAsset_options_resultHandler(
                &asset,
                Some(&options),
                &block,
            );
    }
}

// --- Health -----------------------------------------------------------------

/// The app's one health store (thread-safe in `HealthKit`).
struct Store(Retained<HKHealthStore>);
// SAFETY: HKHealthStore is documented thread-safe; it is only messaged.
unsafe impl Send for Store {}
// SAFETY: as above.
unsafe impl Sync for Store {}

fn store() -> Option<&'static HKHealthStore> {
    static STORE: OnceLock<Option<Store>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            // SAFETY: HealthKit's class methods and initialiser.
            unsafe { HKHealthStore::isHealthDataAvailable().then(|| Store(HKHealthStore::new())) }
        })
        .as_ref()
        .map(|s| &*s.0)
}

fn quantity_type(of: &HealthType) -> Option<Retained<HKQuantityType>> {
    // HealthKit's identifier constants are their own names.
    // SAFETY: HealthKit's lookup.
    unsafe { HKObjectType::quantityTypeForIdentifier(&NSString::from_str(of.healthkit)) }
}

fn unit(of: &HealthType) -> Retained<HKUnit> {
    // SAFETY: HealthKit's unit parser; the table's units are its strings.
    unsafe { HKUnit::unitFromString(&NSString::from_str(of.unit)) }
}

/// `HealthKit`'s permission for `of` (`write`: to save it), then `then`.
fn health_allowed<T: Send + 'static>(
    store: &'static HKHealthStore,
    qt: &HKQuantityType,
    write: bool,
    reply: Arc<Once<T>>,
    then: impl Fn(Arc<Once<T>>) + 'static,
) {
    let sample: &HKSampleType = qt;
    let object: &HKObjectType = sample;
    let share = NSSet::from_slice(&[sample]);
    let read = NSSet::from_slice(&[object]);
    let block = RcBlock::new(move |done: Bool, error: *mut NSError| {
        if done.as_bool() {
            then(Arc::clone(&reply));
        } else {
            reply.send(Err(failed(error_text(error))));
        }
    });
    // SAFETY: HealthKit's request; the block runs once, on its queue.
    unsafe {
        store.requestAuthorizationToShareTypes_readTypes_completion(
            write.then_some(&*share),
            (!write).then_some(&*read),
            &block,
        );
    }
}

fn read_health(
    store: &'static HKHealthStore,
    of: &'static HealthType,
    qt: &HKQuantityType,
    query: &HealthQuery,
    reply: Arc<Once<HealthAnswer>>,
) {
    let unit = unit(of);
    // SAFETY: HealthKit's predicate and queries; each handler runs once.
    unsafe {
        let predicate = HKQuery::predicateForSamplesWithStartDate_endDate_options(
            Some(&date(query.from_ms)),
            Some(&date(query.to_ms)),
            HKQueryOptions::StrictStartDate,
        );
        let (kind, unit_name) = (of.name.to_owned(), of.unit.to_owned());
        if query.wants_sum(of) {
            let block = RcBlock::new(
                move |_q: std::ptr::NonNull<HKStatisticsQuery>,
                      stats: *mut HKStatistics,
                      error: *mut NSError| {
                    // No samples in the span: HealthKit answers an error
                    // ("no data"), which is a sum of 0.
                    let sum = stats
                        .as_ref()
                        .and_then(|s| s.sumQuantity())
                        .map_or(0.0, |q| q.doubleValueForUnit(&unit));
                    let _ = error;
                    reply.send(Ok(HealthAnswer::Sum {
                        kind: kind.clone(),
                        unit: unit_name.clone(),
                        sum,
                    }));
                },
            );
            let q = HKStatisticsQuery::initWithQuantityType_quantitySamplePredicate_options_completionHandler(
                HKStatisticsQuery::alloc(),
                qt,
                Some(&predicate),
                HKStatisticsOptions::CumulativeSum,
                &block,
            );
            store.executeQuery(&q);
        } else {
            let by = NSSortDescriptor::sortDescriptorWithKey_ascending(
                Some(&NSString::from_str("startDate")),
                true,
            );
            let block = RcBlock::new(
                move |_q: std::ptr::NonNull<HKSampleQuery>,
                      found: *mut NSArray<HKSample>,
                      error: *mut NSError| {
                    let Some(found) = found.as_ref() else {
                        reply.send(Err(failed(error_text(error))));
                        return;
                    };
                    let samples = found
                        .iter()
                        .filter_map(|s| s.downcast::<HKQuantitySample>().ok())
                        .map(|s| HealthSample {
                            value: s.quantity().doubleValueForUnit(&unit),
                            start_ms: ms(&s.startDate()),
                            end_ms: ms(&s.endDate()),
                        })
                        .collect();
                    reply.send(Ok(HealthAnswer::Samples {
                        kind: kind.clone(),
                        unit: unit_name.clone(),
                        samples,
                    }));
                },
            );
            let q =
                HKSampleQuery::initWithSampleType_predicate_limit_sortDescriptors_resultsHandler(
                    HKSampleQuery::alloc(),
                    qt,
                    Some(&predicate),
                    0,
                    Some(&NSArray::from_retained_slice(&[by])),
                    &block,
                );
            store.executeQuery(&q);
        }
    }
}

fn write_health(
    store: &'static HKHealthStore,
    of: &'static HealthType,
    qt: &HKQuantityType,
    sample: &HealthSample,
    reply: Arc<Once<()>>,
) {
    // SAFETY: HealthKit's constructors and save; the block runs once.
    unsafe {
        let quantity = HKQuantity::quantityWithUnit_doubleValue(&unit(of), sample.value);
        let saved = HKQuantitySample::quantitySampleWithType_quantity_startDate_endDate(
            qt,
            &quantity,
            &date(sample.start_ms),
            &date(sample.end_ms),
        );
        let block = RcBlock::new(move |done: Bool, error: *mut NSError| {
            reply.send(if done.as_bool() {
                Ok(())
            } else {
                Err(failed(error_text(error)))
            });
        });
        store.saveObject_withCompletion(&saved, &block);
    }
}

impl Device for IosDevice {
    fn notify(&self, key: &str, notification: &OsNotification) -> Asked<()> {
        let key = key.to_owned();
        let n = notification.clone();
        asked(move |reply| {
            let center = center();
            let adding = center.clone();
            let done = Arc::clone(&reply);
            authorized(&center, reply, move || {
                let content = UNMutableNotificationContent::new();
                content.setTitle(&NSString::from_str(&n.title));
                if let Some(body) = &n.body {
                    content.setBody(&NSString::from_str(body));
                }
                content.setSound(Some(&UNNotificationSound::defaultSound()));
                let trigger = n.delay_s(crate::host::now_ms()).map(|s| {
                    UNTimeIntervalNotificationTrigger::triggerWithTimeInterval_repeats(s, false)
                });
                let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                    &NSString::from_str(&key),
                    &content,
                    trigger.as_deref().map(|t| -> &UNNotificationTrigger { t }),
                );
                let done = Arc::clone(&done);
                let added = RcBlock::new(move |error: *mut NSError| {
                    done.send(if error.is_null() {
                        Ok(())
                    } else {
                        Err(failed(error_text(error)))
                    });
                });
                adding.addNotificationRequest_withCompletionHandler(&request, Some(&added));
            });
        })
    }

    fn cancel_notification(&self, key: &str) -> Asked<bool> {
        let key = NSString::from_str(key);
        asked(move |reply| {
            let center = center();
            let removing = center.clone();
            let pending = RcBlock::new(
                move |requests: std::ptr::NonNull<NSArray<UNNotificationRequest>>| {
                    // SAFETY: the handler's array is live for the call.
                    let requests = unsafe { requests.as_ref() };
                    let had = requests
                        .iter()
                        .any(|r| r.identifier().isEqualToString(&key));
                    let ids = NSArray::from_retained_slice(&[key.copy()]);
                    removing.removePendingNotificationRequestsWithIdentifiers(&ids);
                    removing.removeDeliveredNotificationsWithIdentifiers(&ids);
                    reply.send(Ok(had));
                },
            );
            center.getPendingNotificationRequestsWithCompletionHandler(&pending);
        })
    }

    fn capture(&self, _capture: &Capture) -> Asked<Option<String>> {
        asked(|reply| on_main(move |mtm| take_photo(mtm, reply)))
    }

    fn photos(&self, query: &PhotoQuery) -> Asked<Vec<PhotoAsset>> {
        let query = query.clone();
        asked(move |reply| {
            photos_allowed(reply, move |reply| reply.send(Ok(newest(&query))));
        })
    }

    fn photo_file(&self, id: &str) -> Asked<Option<String>> {
        let id = id.to_owned();
        asked(move |reply| photos_allowed(reply, move |reply| photo_copy(&id, reply)))
    }

    fn health_read(&self, of: &'static HealthType, query: &HealthQuery) -> Asked<HealthAnswer> {
        let query = query.clone();
        asked(move |reply| {
            let (Some(store), Some(qt)) = (store(), quantity_type(of)) else {
                reply.send(Err(unsupported("health data on this device")));
                return;
            };
            let kept = qt.clone();
            health_allowed(store, &qt, false, reply, move |reply| {
                read_health(store, of, &kept, &query, reply);
            });
        })
    }

    fn health_write(&self, of: &'static HealthType, sample: &HealthSample) -> Asked<()> {
        let sample = sample.clone();
        debug_assert!(matches!(of.aggregate, Aggregate::Sum | Aggregate::Discrete));
        asked(move |reply| {
            let (Some(store), Some(qt)) = (store(), quantity_type(of)) else {
                reply.send(Err(unsupported("health data on this device")));
                return;
            };
            let kept = qt.clone();
            health_allowed(store, &qt, true, reply, move |reply| {
                write_health(store, of, &kept, &sample, reply);
            });
        })
    }
}
