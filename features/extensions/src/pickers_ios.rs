//! An extension's `pick` on iOS (#708): `UIKit`'s own pickers, presented
//! over the app's window from the main thread.
//!
//! - **Files** (`open`, `open_many`): `UIDocumentPickerViewController`
//!   for opening, as a copy (`asCopy`), so the file is the app's to read
//!   for as long as it likes: no security scope to hold. A filter's
//!   extensions become the content types it shows.
//! - **A folder** (`folder`): the document picker for folders, not a
//!   copy. Its security scope is opened and kept for the run, so the
//!   extension reads and writes in it.
//! - **Save** (`save`): iOS saves by exporting a file that exists, so a
//!   save is a folder picked and the suggested name in it (`Untitled`
//!   without one): the path it answers is where the extension writes.
//! - **Photos** (`photo`, `photos`): `PHPickerViewController`, which runs
//!   out of the app's process: the person picks without the app being
//!   given the photo library (no permission prompt, no usage string).
//!   Each image is copied to the app's temporary folder
//!   (`<tmp>/architect-picks/`) and that copy's path answered.
//!
//! Cancelling (the button, or a swipe down) answers `None`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use block2::RcBlock;
use futures::channel::oneshot;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_foundation::{NSArray, NSError, NSString, NSURL};
use objc2_photos_ui::{
    PHPickerConfiguration, PHPickerConfigurationAssetRepresentationMode, PHPickerFilter,
    PHPickerResult,
};
use objc2_ui_kit::{
    UIApplication, UIDocumentPickerDelegate, UIDocumentPickerViewController, UIViewController,
    UIWindowScene,
};
use objc2_uniform_type_identifiers::{UTType, UTTypeFolder, UTTypeImage, UTTypeItem};

use crate::hostcall::{HostError, Pick, PickKind};
use crate::hostui::Asked;

/// What a picker answers: the paths, `None` when cancelled, or why it
/// could not be shown.
type Picked = Result<Option<Vec<String>>, String>;
type Reply = oneshot::Sender<Picked>;

/// The person's pick, asked on the main thread.
pub fn pick(owner: &str, pick: Pick) -> Asked<Option<Vec<String>>> {
    let owner = owner.to_owned();
    let (tx, rx) = oneshot::channel::<Picked>();
    dispatch2::DispatchQueue::main().exec_async(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = tx.send(Err("not on the main thread".to_owned()));
            return;
        };
        if let Err((tx, why)) = present(mtm, &pick, &owner, tx) {
            let _ = tx.send(Err(why));
        }
    });
    Box::pin(async move {
        match rx.await {
            Ok(Ok(paths)) => Ok(paths),
            Ok(Err(message)) => Err(HostError::Failed { message }),
            // The picker went without answering: as a cancel.
            Err(_) => Ok(None),
        }
    })
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

fn present(
    mtm: MainThreadMarker,
    pick: &Pick,
    owner: &str,
    reply: Reply,
) -> Result<(), (Reply, String)> {
    let Some(over) = top(mtm) else {
        return Err((
            reply,
            "the app has no window to show a picker over".to_owned(),
        ));
    };
    if pick.kind.photos() {
        present_photos(mtm, &over, pick, reply)
    } else {
        present_documents(mtm, &over, pick, owner, reply);
        Ok(())
    }
}

// --- Keeping delegates alive -----------------------------------------------

thread_local! {
    /// The delegates of the pickers showing: a picker holds its delegate
    /// weakly, so they are kept here until they answer.
    static KEPT: RefCell<Vec<Retained<NSObject>>> = const { RefCell::new(Vec::new()) };
}

fn keep(delegate: &NSObject) {
    KEPT.with(|k| k.borrow_mut().push(delegate.retain()));
}

fn release(delegate: &NSObject) {
    KEPT.with(|k| {
        k.borrow_mut()
            .retain(|d| !std::ptr::eq(&raw const **d, delegate));
    });
}

// --- Security scopes ---------------------------------------------------------

/// A folder's URL whose security scope is open for an extension.
struct Scope {
    owner: String,
    url: Retained<NSURL>,
}

// SAFETY: NSURL is immutable and thread-safe; it is only messaged to stop
// its scope, which may be done from any thread.
unsafe impl Send for Scope {}

fn scopes() -> &'static Mutex<Vec<Scope>> {
    static SCOPES: std::sync::OnceLock<Mutex<Vec<Scope>>> = std::sync::OnceLock::new();
    SCOPES.get_or_init(|| Mutex::new(Vec::new()))
}

fn keep_scope(owner: &str, url: &NSURL) {
    scopes()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Scope {
            owner: owner.to_owned(),
            url: url.retain(),
        });
}

/// Stops the security scopes opened for extension `owner`'s picks: it
/// unloaded, and the folders it was given are closed again.
pub fn stop_scopes(owner: &str) {
    let gone: Vec<Scope> = {
        let mut all = scopes().lock().unwrap_or_else(PoisonError::into_inner);
        let (gone, kept) = std::mem::take(&mut *all)
            .into_iter()
            .partition(|s| s.owner == owner);
        *all = kept;
        gone
    };
    for s in gone {
        // SAFETY: a URL whose scope this module opened.
        unsafe { s.url.stopAccessingSecurityScopedResource() };
    }
}

// --- The document picker ---------------------------------------------------

/// The content types a document pick shows.
fn content_types(pick: &Pick) -> Retained<NSArray<UTType>> {
    // SAFETY: the statics are UniformTypeIdentifiers' constants.
    let (item, folder) = unsafe { (UTTypeItem, UTTypeFolder) };
    if matches!(pick.kind, PickKind::Folder | PickKind::Save) {
        return NSArray::from_slice(&[folder]);
    }
    let types: Vec<Retained<UTType>> = pick
        .filters
        .iter()
        .flat_map(|f| f.extensions.iter())
        .filter_map(|ext| UTType::typeWithFilenameExtension(&NSString::from_str(ext)))
        .collect();
    if types.is_empty() {
        NSArray::from_slice(&[item])
    } else {
        NSArray::from_retained_slice(&types)
    }
}

struct DocumentIvars {
    reply: RefCell<Option<Reply>>,
    kind: PickKind,
    name: Option<String>,
    /// The extension asking (its scopes stop when it unloads).
    owner: String,
}

define_class!(
    // SAFETY: an NSObject subclass with no superclass method overridden;
    // its ivars are only touched on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ArchitectExtensionDocumentPicker"]
    #[ivars = DocumentIvars]
    struct DocumentDelegate;

    unsafe impl NSObjectProtocol for DocumentDelegate {}

    unsafe impl UIDocumentPickerDelegate for DocumentDelegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn did_pick(&self, _picker: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
            let paths = self.paths(urls);
            self.answer(Ok(Some(paths)));
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _picker: &UIDocumentPickerViewController) {
            self.answer(Ok(None));
        }
    }
);

impl DocumentDelegate {
    fn new(mtm: MainThreadMarker, pick: &Pick, owner: &str, reply: Reply) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DocumentIvars {
            reply: RefCell::new(Some(reply)),
            kind: pick.kind,
            name: pick.name.clone(),
            owner: owner.to_owned(),
        });
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![super(this), init] }
    }

    fn paths(&self, urls: &NSArray<NSURL>) -> Vec<String> {
        let kind = self.ivars().kind;
        urls.iter()
            .filter_map(|url| {
                if matches!(kind, PickKind::Folder | PickKind::Save) {
                    // Outside the app's container: its scope opened, and
                    // kept until the extension unloads ([`stop_scopes`]):
                    // it reads and writes in it after this answers.
                    // SAFETY: a file URL the picker handed the app.
                    if unsafe { url.startAccessingSecurityScopedResource() } {
                        keep_scope(&self.ivars().owner, &url);
                    }
                }
                let path = PathBuf::from(url.path()?.to_string());
                Some(match kind {
                    // One plain file name in the folder picked: an
                    // extension's `name` never leaves it.
                    PickKind::Save => {
                        crate::hostcall::save_path(&path, self.ivars().name.as_deref())
                            .display()
                            .to_string()
                    }
                    _ => path.display().to_string(),
                })
            })
            .collect()
    }

    fn answer(&self, picked: Picked) {
        if let Some(tx) = self.ivars().reply.borrow_mut().take() {
            let _ = tx.send(picked);
        }
        release(self);
    }
}

fn present_documents(
    mtm: MainThreadMarker,
    over: &UIViewController,
    pick: &Pick,
    owner: &str,
    reply: Reply,
) {
    let types = content_types(pick);
    // A file is the app's own copy; a folder (and a save's) the person's,
    // reached through its scope.
    let copy = matches!(pick.kind, PickKind::Open | PickKind::OpenMany);
    let picker = UIDocumentPickerViewController::initForOpeningContentTypes_asCopy(
        UIDocumentPickerViewController::alloc(mtm),
        &types,
        copy,
    );
    picker.setAllowsMultipleSelection(pick.kind == PickKind::OpenMany);
    if let Some(folder) = &pick.folder {
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(folder), true);
        picker.setDirectoryURL(Some(&url));
    }
    let delegate = DocumentDelegate::new(mtm, pick, owner, reply);
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    keep(&delegate);
    over.presentViewController_animated_completion(&picker, true, None);
}

// --- The photo picker ------------------------------------------------------

/// Where picked photos are copied: the app's temporary folder.
fn picks_dir() -> PathBuf {
    std::env::temp_dir().join("architect-picks")
}

/// The provider's image (`from`, gone once its handler returns) as a
/// plain JPEG in [`picks_dir`].
fn copy_in(from: &Path) -> Option<String> {
    let image = objc2_ui_kit::UIImage::imageWithContentsOfFile(&NSString::from_str(
        &from.display().to_string(),
    ))?;
    plain_jpeg(&image, &picks_dir())
}

/// `image` written as a JPEG in `dir`, its path: the pixels alone, drawn
/// again, so none of the original's metadata goes with it (EXIF, the
/// camera, the place it was taken: GPS). An extension is handed only
/// these copies (`pick {kind: "photo"}`, `photo_file`).
pub fn plain_jpeg(image: &objc2_ui_kit::UIImage, dir: &Path) -> Option<String> {
    std::fs::create_dir_all(dir).ok()?;
    let data = image.jpeg_representation(0.9)?;
    let name = format!(
        "{}-{}.jpg",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
        std::process::id()
    );
    let to = dir.join(name);
    std::fs::write(&to, data.to_vec()).ok()?;
    Some(to.display().to_string())
}

struct PhotoIvars {
    reply: RefCell<Option<Reply>>,
}

define_class!(
    // SAFETY: as `DocumentDelegate`. It answers PHPickerViewControllerDelegate's
    // one method, which UIKit finds by its selector.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ArchitectExtensionPhotoPicker"]
    #[ivars = PhotoIvars]
    struct PhotoDelegate;

    unsafe impl NSObjectProtocol for PhotoDelegate {}

    impl PhotoDelegate {
        #[unsafe(method(picker:didFinishPicking:))]
        fn did_finish(&self, picker: &UIViewController, results: &NSArray<PHPickerResult>) {
            picker.dismissViewControllerAnimated_completion(true, None);
            let reply = self.ivars().reply.borrow_mut().take();
            release(self);
            let Some(reply) = reply else { return };
            if results.is_empty() {
                let _ = reply.send(Ok(None));
                return;
            }
            load_all(results, reply);
        }
    }
);

impl PhotoDelegate {
    fn new(mtm: MainThreadMarker, reply: Reply) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PhotoIvars {
            reply: RefCell::new(Some(reply)),
        });
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![super(this), init] }
    }
}

/// Each result's image, as a file copied in; the reply once all are.
fn load_all(results: &NSArray<PHPickerResult>, reply: Reply) {
    struct Gathering {
        left: usize,
        paths: Vec<Option<String>>,
        reply: Option<Reply>,
    }
    let n = results.count();
    let gathering = Arc::new(Mutex::new(Gathering {
        left: n,
        paths: vec![None; n],
        reply: Some(reply),
    }));
    // SAFETY: UniformTypeIdentifiers' constant.
    let image = unsafe { UTTypeImage }.identifier();
    for (i, result) in results.iter().enumerate() {
        // SAFETY: a result's provider, read on the main thread.
        let provider = unsafe { result.itemProvider() };
        let gathering = Arc::clone(&gathering);
        let done = RcBlock::new(move |url: *mut NSURL, _error: *mut NSError| {
            // SAFETY: the handler's URL is nil or a live file URL for
            // the length of this call.
            let copied = unsafe { url.as_ref() }
                .and_then(NSURL::path)
                .and_then(|p| copy_in(Path::new(&p.to_string())));
            let finished = {
                let mut g = gathering.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(slot) = g.paths.get_mut(i) {
                    *slot = copied;
                }
                g.left = g.left.saturating_sub(1);
                if g.left == 0 {
                    g.reply
                        .take()
                        .map(|tx| (tx, g.paths.drain(..).flatten().collect::<Vec<String>>()))
                } else {
                    None
                }
            };
            if let Some((tx, paths)) = finished {
                let _ = tx.send(if paths.is_empty() {
                    Err("the photos picked could not be read".to_owned())
                } else {
                    Ok(Some(paths))
                });
            }
        });
        // SAFETY: the block is Send (its state behind a Mutex in an Arc)
        // and is called once, on a queue of the provider's.
        let _progress = unsafe {
            provider.loadFileRepresentationForTypeIdentifier_completionHandler(&image, &done)
        };
    }
}

fn present_photos(
    mtm: MainThreadMarker,
    over: &UIViewController,
    pick: &Pick,
    reply: Reply,
) -> Result<(), (Reply, String)> {
    let Some(class) = AnyClass::get(c"PHPickerViewController") else {
        return Err((reply, "this iOS has no photo picker".to_owned()));
    };
    // SAFETY: PHPickerConfiguration's own initialiser and setters; the
    // images filter is PhotosUI's.
    let config = unsafe {
        let config = PHPickerConfiguration::new();
        config.setFilter(Some(&PHPickerFilter::imagesFilter()));
        // 0: as many as the person likes.
        config.setSelectionLimit(isize::from(pick.kind != PickKind::Photos));
        config.setPreferredAssetRepresentationMode(
            PHPickerConfigurationAssetRepresentationMode::Compatible,
        );
        config
    };
    // SAFETY: PHPickerViewController is a UIViewController whose
    // designated initialiser is `initWithConfiguration:`; its delegate
    // property takes any object answering `picker:didFinishPicking:`.
    let picker: Retained<UIViewController> = unsafe {
        let alloc: Allocated<UIViewController> = msg_send![class, alloc];
        msg_send![alloc, initWithConfiguration: &*config]
    };
    let delegate = PhotoDelegate::new(mtm, reply);
    // SAFETY: as above.
    let () = unsafe { msg_send![&*picker, setDelegate: &*delegate] };
    keep(&delegate);
    over.presentViewController_animated_completion(&picker, true, None);
    Ok(())
}
