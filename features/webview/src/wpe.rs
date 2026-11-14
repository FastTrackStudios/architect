//! WPE WebKit, off-screen (feature `wpe`, Linux): the web view kind on a
//! native Wayland desk.
//!
//! WebKitGTK can be a child only of an X11 window; WPE WebKit renders a page
//! with no window at all, through the WPE Platform API, and hands each frame
//! out as a dmabuf (rendered on the GPU) or shared memory (Skia on the CPU).
//! `glue.c` is our own WPE Platform display, toplevel and view, the package's
//! protocol, the `architect` script-message handler the runtime posts to,
//! and input in; this module is its safe face, [`WpeWeb`], one per pane, on
//! the window's thread. The pane's widget (`crate::wpe_pane`) draws its
//! frames as a texture, so the app's overlays draw over it as over anything.
//!
//! Spike and measurements: `docs/content/architecture/embedded-views.md`,
//! "Web views".

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::rc::Rc;

#[allow(non_camel_case_types, dead_code)]
mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub struct ArchWeb {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct ArchFrame {
        pub id: u64,
        pub kind: c_int,
        pub width: c_int,
        pub height: c_int,
        pub fourcc: u32,
        pub modifier: u64,
        pub n_planes: c_int,
        pub fds: [c_int; 4],
        pub offsets: [u32; 4],
        pub strides: [u32; 4],
        pub pixels: *const u8,
        pub size: usize,
    }

    #[repr(C)]
    pub struct ArchResponse {
        pub status: c_int,
        pub mime: *const c_char,
        pub body: *const u8,
        pub len: usize,
        pub headers: *const c_char,
    }

    #[repr(C)]
    pub struct ArchCallbacks {
        pub user: *mut c_void,
        pub frame: Option<unsafe extern "C" fn(*mut c_void, *const ArchFrame)>,
        pub message: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
        pub scheme: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut ArchResponse) -> c_int>,
        pub cursor: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
        pub ime_area: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int)>,
        pub loaded: Option<unsafe extern "C" fn(*mut c_void, c_int)>,
        pub crashed: Option<unsafe extern "C" fn(*mut c_void, c_int)>,
    }

    unsafe extern "C" {
        pub fn arch_web_new(
            primary_node: *const c_char,
            render_node: *const c_char,
            width: c_int,
            height: c_int,
            scale: f64,
            scheme: *const c_char,
            boot_js: *const c_char,
            callbacks: ArchCallbacks,
            error: *mut *mut c_char,
        ) -> *mut ArchWeb;
        pub fn arch_web_free(web: *mut ArchWeb);
        pub fn arch_web_free_string(s: *mut c_char);
        pub fn arch_web_load(web: *mut ArchWeb, uri: *const c_char);
        pub fn arch_web_reload(web: *mut ArchWeb);
        pub fn arch_web_resize(web: *mut ArchWeb, width: c_int, height: c_int, scale: f64);
        pub fn arch_web_eval(web: *mut ArchWeb, js: *const c_char);
        pub fn arch_web_frame_done(web: *mut ArchWeb, id: u64);
        pub fn arch_web_set_visible(web: *mut ArchWeb, visible: c_int);
        pub fn arch_web_pointer(
            web: *mut ArchWeb,
            kind: c_int,
            x: f64,
            y: f64,
            button: u32,
            mods: u32,
            press_count: u32,
            touch: c_int,
        );
        pub fn arch_web_scroll(web: *mut ArchWeb, x: f64, y: f64, dx: f64, dy: f64, precise: c_int, mods: u32);
        pub fn arch_web_key(web: *mut ArchWeb, down: c_int, keyval: u32, keycode: u32, mods: u32);
        pub fn arch_web_focus(web: *mut ArchWeb, focused: c_int);
        pub fn arch_web_ime_commit(web: *mut ArchWeb, text: *const c_char);
        pub fn arch_web_ime_preedit(web: *mut ArchWeb, text: *const c_char, cursor: u32);
        pub fn arch_web_iterate() -> c_int;
    }
}

/// A frame of the page, as WPE rendered it.
#[derive(Debug)]
pub enum FramePixels {
    /// On the GPU: a dmabuf per plane (borrowed until the frame is done).
    DmaBuf {
        fourcc: u32,
        modifier: u64,
        /// `(fd, offset, stride)` per plane.
        planes: Vec<(i32, u32, u32)>,
    },
    /// Copied out of WPE's shared memory: BGRA, premultiplied.
    Shm { pixels: Vec<u8>, stride: u32 },
}

/// One frame: its id (for [`WpeWeb::frame_done`]), its size in pixels.
#[derive(Debug)]
pub struct Frame {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: FramePixels,
}

/// A package file, as the protocol answers it.
pub struct Answer {
    pub status: u16,
    pub mime: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

/// What a view tells its pane.
#[derive(Default)]
pub struct Inbox {
    /// The newest frame not drawn yet.
    pub frame: Option<Frame>,
    /// The bridge's messages from the page.
    pub messages: Vec<String>,
    /// The cursor the page asks for (a CSS cursor name).
    pub cursor: Option<String>,
    /// Where the page's caret is, for the IME's candidate window.
    pub ime_area: Option<(i32, i32, i32, i32)>,
    /// The web process ended (its reason).
    pub crashed: Option<i32>,
    /// Loads finished (a reload sends the theme again).
    pub loads: u32,
}

/// What a view asks its pane for, synchronously.
pub type Protocol = Box<dyn Fn(&str) -> Option<Answer>>;

struct Shared {
    inbox: RefCell<Inbox>,
    protocol: Protocol,
    /// Called on every frame or message: the pane asks for a redraw.
    wake: Box<dyn Fn()>,
    /// The answer being handed to C (copied there before the call returns).
    answer: RefCell<Option<(Answer, CString, CString)>>,
}

/// One WPE web view (on the window's thread).
pub struct WpeWeb {
    raw: *mut ffi::ArchWeb,
    shared: Rc<Shared>,
}

/// How WPE renders: on a GPU's render node (dmabuf frames) or on the CPU
/// (shared memory).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Renderer {
    Gpu { primary: String, render: String },
    Cpu,
}

impl Renderer {
    /// The first render node there is, with its primary node; else the CPU.
    #[must_use]
    pub fn detect() -> Self {
        if std::env::var_os("ARCHITECT_WEBVIEW_CPU").is_some() {
            return Self::Cpu;
        }
        let render = (128..136)
            .map(|n| format!("/dev/dri/renderD{n}"))
            .find(|p| std::path::Path::new(p).exists());
        let primary = (0..8)
            .map(|n| format!("/dev/dri/card{n}"))
            .find(|p| std::path::Path::new(p).exists());
        match (primary, render) {
            (Some(primary), Some(render)) => Self::Gpu { primary, render },
            _ => Self::Cpu,
        }
    }
}

unsafe extern "C" fn on_frame(user: *mut c_void, f: *const ffi::ArchFrame) {
    // SAFETY: `user` is the `Shared` this view's `Rc` keeps alive; `f` is
    // valid for the call.
    let (shared, f) = unsafe { (&*user.cast::<Shared>(), &*f) };
    let pixels = match f.kind {
        1 => FramePixels::DmaBuf {
            fourcc: f.fourcc,
            modifier: f.modifier,
            planes: (0..usize::try_from(f.n_planes).unwrap_or(0).min(4))
                .filter_map(|p| Some((*f.fds.get(p)?, *f.offsets.get(p)?, *f.strides.get(p)?)))
                .collect(),
        },
        _ if !f.pixels.is_null() => FramePixels::Shm {
            // SAFETY: WPE's shared memory, `size` bytes, valid for the call.
            pixels: unsafe { std::slice::from_raw_parts(f.pixels, f.size) }.to_vec(),
            stride: f.strides[0],
        },
        _ => return,
    };
    let frame = Frame {
        id: f.id,
        width: u32::try_from(f.width).unwrap_or(0),
        height: u32::try_from(f.height).unwrap_or(0),
        pixels,
    };
    shared.inbox.borrow_mut().frame = Some(frame);
    (shared.wake)();
}

unsafe extern "C" fn on_message(user: *mut c_void, json: *const c_char) {
    // SAFETY: as `on_frame`; `json` a NUL-terminated string for the call.
    let (shared, text) = unsafe { (&*user.cast::<Shared>(), CStr::from_ptr(json)) };
    shared
        .inbox
        .borrow_mut()
        .messages
        .push(text.to_string_lossy().into_owned());
    (shared.wake)();
}

unsafe extern "C" fn on_scheme(user: *mut c_void, uri: *const c_char, out: *mut ffi::ArchResponse) -> c_int {
    // SAFETY: as `on_frame`; `out` writable for the call.
    let (shared, uri) = unsafe { (&*user.cast::<Shared>(), CStr::from_ptr(uri)) };
    let Some(answer) = (shared.protocol)(&uri.to_string_lossy()) else {
        return 0;
    };
    let mime = CString::new(answer.mime.clone()).unwrap_or_default();
    let headers = CString::new(
        answer
            .headers
            .iter()
            .flat_map(|(k, v)| [k.as_str(), v.as_str()])
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap_or_default();
    let mut slot = shared.answer.borrow_mut();
    *slot = Some((answer, mime, headers));
    if let Some((answer, mime, headers)) = slot.as_ref() {
        // SAFETY: `out` is the caller's; what it points into lives in
        // `shared.answer` until the next request, and C copies it now.
        unsafe {
            *out = ffi::ArchResponse {
                status: c_int::from(answer.status),
                mime: mime.as_ptr(),
                body: answer.body.as_ptr(),
                len: answer.body.len(),
                headers: headers.as_ptr(),
            };
        }
    }
    1
}

unsafe extern "C" fn on_cursor(user: *mut c_void, name: *const c_char) {
    // SAFETY: as `on_message`.
    let (shared, name) = unsafe { (&*user.cast::<Shared>(), CStr::from_ptr(name)) };
    shared.inbox.borrow_mut().cursor = Some(name.to_string_lossy().into_owned());
    (shared.wake)();
}

unsafe extern "C" fn on_ime_area(user: *mut c_void, x: c_int, y: c_int, w: c_int, h: c_int) {
    // SAFETY: as `on_frame`.
    let shared = unsafe { &*user.cast::<Shared>() };
    shared.inbox.borrow_mut().ime_area = Some((x, y, w, h));
}

unsafe extern "C" fn on_loaded(user: *mut c_void, event: c_int) {
    // SAFETY: as `on_frame`.
    let shared = unsafe { &*user.cast::<Shared>() };
    // WEBKIT_LOAD_FINISHED.
    if event == 3 {
        shared.inbox.borrow_mut().loads += 1;
        (shared.wake)();
    }
}

unsafe extern "C" fn on_crashed(user: *mut c_void, reason: c_int) {
    // SAFETY: as `on_frame`.
    let shared = unsafe { &*user.cast::<Shared>() };
    shared.inbox.borrow_mut().crashed = Some(reason);
    (shared.wake)();
}

/// Modifiers as the glue takes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Mods {
    const fn bits(self) -> u32 {
        (self.control as u32) | (self.shift as u32) << 1 | (self.alt as u32) << 2 | (self.meta as u32) << 3
    }
}

/// A pointer's change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Down = 1,
    Up = 2,
    Move = 3,
    Enter = 4,
    Leave = 5,
}

impl WpeWeb {
    /// A view of `width`×`height` logical pixels rendered by `renderer`, its
    /// documents from `scheme` answered by `protocol`, `boot_js` run at
    /// each document's start; `wake` asks the pane to draw again.
    ///
    /// # Errors
    ///
    /// WPE did not start (no display, a bad scheme name).
    pub fn new(
        renderer: &Renderer,
        size: (u32, u32),
        scheme: &str,
        boot_js: &str,
        protocol: Protocol,
        wake: impl Fn() + 'static,
    ) -> Result<Self, String> {
        let shared = Rc::new(Shared {
            inbox: RefCell::new(Inbox::default()),
            protocol,
            wake: Box::new(wake),
            answer: RefCell::new(None),
        });
        let callbacks = ffi::ArchCallbacks {
            user: Rc::as_ptr(&shared).cast_mut().cast(),
            frame: Some(on_frame),
            message: Some(on_message),
            scheme: Some(on_scheme),
            cursor: Some(on_cursor),
            ime_area: Some(on_ime_area),
            loaded: Some(on_loaded),
            crashed: Some(on_crashed),
        };
        let cstr = |s: &str| CString::new(s).map_err(|_| format!("{s:?} has a NUL"));
        let (primary, render) = match renderer {
            Renderer::Gpu { primary, render } => (Some(cstr(primary)?), Some(cstr(render)?)),
            Renderer::Cpu => (None, None),
        };
        let (scheme, boot) = (cstr(scheme)?, cstr(boot_js)?);
        let mut error: *mut c_char = std::ptr::null_mut();
        let (w, h) = (
            c_int::try_from(size.0).unwrap_or(1),
            c_int::try_from(size.1).unwrap_or(1),
        );
        // SAFETY: every pointer is valid for the call; `callbacks.user` is
        // kept alive by `shared`, which outlives `raw` (Drop frees `raw`
        // first).
        let raw = unsafe {
            ffi::arch_web_new(
                primary.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
                render.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
                w,
                h,
                1.0,
                scheme.as_ptr(),
                boot.as_ptr(),
                callbacks,
                &raw mut error,
            )
        };
        if raw.is_null() {
            let why = if error.is_null() {
                "WPE WebKit did not start".to_owned()
            } else {
                // SAFETY: a string the glue allocated, freed here.
                let s = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
                unsafe { ffi::arch_web_free_string(error) };
                s
            };
            return Err(why);
        }
        Ok(Self { raw, shared })
    }

    /// What it has told the pane since it last looked.
    pub fn take(&self) -> Inbox {
        std::mem::take(&mut *self.shared.inbox.borrow_mut())
    }

    pub fn load(&self, uri: &str) {
        if let Ok(uri) = CString::new(uri) {
            // SAFETY: `raw` is live while `self` is.
            unsafe { ffi::arch_web_load(self.raw, uri.as_ptr()) };
        }
    }

    pub fn reload(&self) {
        // SAFETY: as `load`.
        unsafe { ffi::arch_web_reload(self.raw) };
    }

    pub fn resize(&self, width: u32, height: u32) {
        // SAFETY: as `load`.
        unsafe {
            ffi::arch_web_resize(
                self.raw,
                c_int::try_from(width).unwrap_or(1),
                c_int::try_from(height).unwrap_or(1),
                1.0,
            );
        }
    }

    pub fn eval(&self, js: &str) {
        if let Ok(js) = CString::new(js) {
            // SAFETY: as `load`.
            unsafe { ffi::arch_web_eval(self.raw, js.as_ptr()) };
        }
    }

    /// Frame `id` is shown: WebKit may render the next.
    pub fn frame_done(&self, id: u64) {
        // SAFETY: as `load`.
        unsafe { ffi::arch_web_frame_done(self.raw, id) };
    }

    pub fn set_visible(&self, visible: bool) {
        // SAFETY: as `load`.
        unsafe { ffi::arch_web_set_visible(self.raw, c_int::from(visible)) };
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pointer(&self, kind: PointerKind, x: f64, y: f64, button: u32, mods: Mods, count: u32, touch: bool) {
        // SAFETY: as `load`.
        unsafe {
            ffi::arch_web_pointer(self.raw, kind as c_int, x, y, button, mods.bits(), count, c_int::from(touch));
        }
    }

    pub fn scroll(&self, x: f64, y: f64, dx: f64, dy: f64, precise: bool, mods: Mods) {
        // SAFETY: as `load`.
        unsafe { ffi::arch_web_scroll(self.raw, x, y, dx, dy, c_int::from(precise), mods.bits()) };
    }

    pub fn key(&self, down: bool, keysym: u32, mods: Mods) {
        // SAFETY: as `load`.
        unsafe { ffi::arch_web_key(self.raw, c_int::from(down), keysym, 0, mods.bits()) };
    }

    pub fn focus(&self, focused: bool) {
        // SAFETY: as `load`.
        unsafe { ffi::arch_web_focus(self.raw, c_int::from(focused)) };
    }

    pub fn ime_commit(&self, text: &str) {
        if let Ok(text) = CString::new(text) {
            // SAFETY: as `load`.
            unsafe { ffi::arch_web_ime_commit(self.raw, text.as_ptr()) };
        }
    }

    pub fn ime_preedit(&self, text: &str, cursor: u32) {
        if let Ok(text) = CString::new(text) {
            // SAFETY: as `load`.
            unsafe { ffi::arch_web_ime_preedit(self.raw, text.as_ptr(), cursor) };
        }
    }
}

impl Drop for WpeWeb {
    fn drop(&mut self) {
        // SAFETY: `raw` is ours and live; freed once.
        unsafe { ffi::arch_web_free(self.raw) };
    }
}

/// Runs what WPE has due on this thread's GLib main context, without
/// waiting: whether anything ran.
pub fn iterate() -> bool {
    // SAFETY: no arguments; GLib's default context on this thread.
    unsafe { ffi::arch_web_iterate() != 0 }
}

/// An X keysym for a key, as WPE takes it: a named key's, or a
/// character's (Latin-1 as itself, the rest as `0x0100_0000 + code`).
#[must_use]
pub fn keysym(named: Option<&str>, text: Option<&str>) -> Option<u32> {
    let named = match named? {
        "Enter" => 0xff0d,
        "Escape" => 0xff1b,
        "Backspace" => 0xff08,
        "Tab" => 0xff09,
        "Delete" => 0xffff,
        "Home" => 0xff50,
        "End" => 0xff57,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        "Insert" => 0xff63,
        "Shift" => 0xffe1,
        "Control" => 0xffe3,
        "Alt" => 0xffe9,
        "Meta" | "Super" => 0xffeb,
        "CapsLock" => 0xffe5,
        "F1" => 0xffbe,
        "F2" => 0xffbf,
        "F3" => 0xffc0,
        "F4" => 0xffc1,
        "F5" => 0xffc2,
        "F6" => 0xffc3,
        "F7" => 0xffc4,
        "F8" => 0xffc5,
        "F9" => 0xffc6,
        "F10" => 0xffc7,
        "F11" => 0xffc8,
        "F12" => 0xffc9,
        _ => 0,
    };
    if named != 0 {
        return Some(named);
    }
    let c = text?.chars().next()?;
    let code = u32::from(c);
    Some(if (0x20..=0x7e).contains(&code) || (0xa0..=0xff).contains(&code) {
        code
    } else {
        0x0100_0000 + code
    })
}

#[cfg(test)]
mod tests {
    use super::keysym;

    #[test]
    fn keys_are_x_keysyms() {
        assert_eq!(keysym(Some("Enter"), None), Some(0xff0d));
        assert_eq!(keysym(Some("Character"), Some("a")), Some(u32::from('a')));
        assert_eq!(keysym(Some("Character"), Some("é")), Some(0xe9));
        assert_eq!(keysym(Some("Character"), Some("€")), Some(0x0100_20ac));
        assert_eq!(keysym(Some("Unidentified"), None), None);
    }
}
