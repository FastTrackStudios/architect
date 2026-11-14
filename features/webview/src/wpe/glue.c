/* The WPE WebKit glue (architect-webview, feature `wpe`): see glue.h.
 *
 * Our own WPE Platform display, toplevel and view: WebKit renders the page
 * off-screen (GBM on the render node, or Skia on the CPU into shared memory)
 * and hands each frame to ArchView::render_buffer; the app draws it in its
 * pane and says when it was shown (arch_web_frame_done), which paces WebKit
 * to the app's own frames. Spike and measurements:
 * docs/content/architecture/embedded-views.md, "Web views". */
#include "glue.h"

#include <epoxy/egl.h>
#include <fcntl.h>
#include <gbm.h>
#include <string.h>
#include <unistd.h>
#include <wpe/webkit.h>
#include <wpe/wpe-platform.h>

struct ArchWeb {
    ArchCallbacks cb;
    WPEDisplay *display;
    WebKitWebView *web;
    WPEView *view;
    WPEToplevel *toplevel;
    WPEInputMethodContext *ime;
    WPEBuffer *pending, *shown;
    uint64_t pending_id, next_id;
    char *scheme;
};

/* ── the input method context: what the app's IME composes ─────────────── */
typedef struct { WPEInputMethodContext parent; ArchWeb *web; char *preedit; guint cursor; } ArchIme;
typedef struct { WPEInputMethodContextClass parent; } ArchImeClass;
G_DEFINE_TYPE(ArchIme, arch_ime, WPE_TYPE_INPUT_METHOD_CONTEXT)
static void arch_ime_get_preedit(WPEInputMethodContext *c, gchar **text, GList **underlines, guint *cursor) {
    ArchIme *i = (ArchIme *)c;
    if (text) *text = g_strdup(i->preedit ? i->preedit : "");
    if (underlines) *underlines = NULL;
    if (cursor) *cursor = i->cursor;
}
static void arch_ime_cursor_area(WPEInputMethodContext *c, int x, int y, int w, int h) {
    ArchIme *i = (ArchIme *)c;
    if (i->web && i->web->cb.ime_area) i->web->cb.ime_area(i->web->cb.user, x, y, w, h);
}
static void arch_ime_finalize(GObject *o) { g_free(((ArchIme *)o)->preedit); G_OBJECT_CLASS(arch_ime_parent_class)->finalize(o); }
static void arch_ime_class_init(ArchImeClass *k) {
    G_OBJECT_CLASS(k)->finalize = arch_ime_finalize;
    WPE_INPUT_METHOD_CONTEXT_CLASS(k)->get_preedit_string = arch_ime_get_preedit;
    WPE_INPUT_METHOD_CONTEXT_CLASS(k)->set_cursor_area = arch_ime_cursor_area;
}
static void arch_ime_init(ArchIme *i) {}

/* ── the view: frames out, cursors out ─────────────────────────────────── */
typedef struct { WPEView parent; ArchWeb *web; } ArchView;
typedef struct { WPEViewClass parent; } ArchViewClass;
G_DEFINE_TYPE(ArchView, arch_view, WPE_TYPE_VIEW)

static gboolean arch_render_buffer(WPEView *view, WPEBuffer *buffer, const WPERectangle *d, guint nd, GError **e) {
    ArchWeb *web = ((ArchView *)view)->web;
    if (!web) return FALSE;
    /* A frame not yet shown is superseded: report it rendered, give it back. */
    if (web->pending) {
        wpe_view_buffer_rendered(view, web->pending);
        wpe_view_buffer_released(view, web->pending);
        g_clear_object(&web->pending);
    }
    web->pending = g_object_ref(buffer);
    web->pending_id = ++web->next_id;
    ArchFrame f = { 0 };
    f.id = web->pending_id;
    f.width = wpe_buffer_get_width(buffer);
    f.height = wpe_buffer_get_height(buffer);
    if (WPE_IS_BUFFER_DMA_BUF(buffer)) {
        WPEBufferDMABuf *b = WPE_BUFFER_DMA_BUF(buffer);
        f.kind = 1;
        f.fourcc = wpe_buffer_dma_buf_get_format(b);
        f.modifier = wpe_buffer_dma_buf_get_modifier(b);
        f.n_planes = (int)wpe_buffer_dma_buf_get_n_planes(b);
        for (int p = 0; p < f.n_planes && p < 4; p++) {
            f.fds[p] = wpe_buffer_dma_buf_get_fd(b, p);
            f.offsets[p] = wpe_buffer_dma_buf_get_offset(b, p);
            f.strides[p] = wpe_buffer_dma_buf_get_stride(b, p);
        }
    } else if (WPE_IS_BUFFER_SHM(buffer)) {
        WPEBufferSHM *b = WPE_BUFFER_SHM(buffer);
        gsize n = 0;
        f.kind = 2;
        f.fourcc = wpe_buffer_shm_get_format(b);
        f.strides[0] = wpe_buffer_shm_get_stride(b);
        f.pixels = g_bytes_get_data(wpe_buffer_shm_get_data(b), &n);
        f.size = n;
    }
    if (web->cb.frame) web->cb.frame(web->cb.user, &f);
    return TRUE;
}

static void arch_set_cursor(WPEView *view, const char *name) {
    ArchWeb *web = ((ArchView *)view)->web;
    if (web && web->cb.cursor) web->cb.cursor(web->cb.user, name);
}
static gboolean arch_can_be_mapped(WPEView *view) { return TRUE; }
static void arch_view_class_init(ArchViewClass *k) {
    WPE_VIEW_CLASS(k)->render_buffer = arch_render_buffer;
    WPE_VIEW_CLASS(k)->set_cursor_from_name = arch_set_cursor;
    WPE_VIEW_CLASS(k)->can_be_mapped = arch_can_be_mapped;
}
static void arch_view_init(ArchView *v) {}

/* ── the toplevel: always active, its size the pane's ──────────────────── */
typedef struct { WPEToplevel parent; } ArchToplevel;
typedef struct { WPEToplevelClass parent; } ArchToplevelClass;
G_DEFINE_TYPE(ArchToplevel, arch_toplevel, WPE_TYPE_TOPLEVEL)
static gboolean resize_view(WPEToplevel *t, WPEView *v, gpointer _) {
    int w, h;
    wpe_toplevel_get_size(t, &w, &h);
    wpe_view_resized(v, w, h);
    return FALSE;
}
static gboolean arch_toplevel_resize(WPEToplevel *t, int w, int h) {
    wpe_toplevel_resized(t, w, h);
    wpe_toplevel_foreach_view(t, resize_view, NULL);
    return TRUE;
}
static void arch_toplevel_constructed(GObject *o) {
    G_OBJECT_CLASS(arch_toplevel_parent_class)->constructed(o);
    wpe_toplevel_state_changed(WPE_TOPLEVEL(o), WPE_TOPLEVEL_STATE_ACTIVE);
}
static void arch_toplevel_class_init(ArchToplevelClass *k) {
    G_OBJECT_CLASS(k)->constructed = arch_toplevel_constructed;
    WPE_TOPLEVEL_CLASS(k)->resize = arch_toplevel_resize;
}
static void arch_toplevel_init(ArchToplevel *t) {}

/* ── the display ──────────────────────────────────────────────────────── */
typedef struct { WPEDisplay parent; WPEDRMDevice *drm; WPEKeymap *keymap; struct gbm_device *gbm; int gbm_fd; } ArchDisplay;
typedef struct { WPEDisplayClass parent; } ArchDisplayClass;
G_DEFINE_TYPE(ArchDisplay, arch_display, WPE_TYPE_DISPLAY)
static gboolean arch_connect(WPEDisplay *d, GError **e) { return TRUE; }
static WPEView *arch_create_view(WPEDisplay *d) { return g_object_new(arch_view_get_type(), "display", d, NULL); }
static WPEToplevel *arch_create_toplevel(WPEDisplay *d, guint max) { return g_object_new(arch_toplevel_get_type(), "display", d, NULL); }
static WPEDRMDevice *arch_get_drm(WPEDisplay *d) { return ((ArchDisplay *)d)->drm; }
static WPEKeymap *arch_get_keymap(WPEDisplay *d) {
    ArchDisplay *a = (ArchDisplay *)d;
    if (!a->keymap) a->keymap = wpe_keymap_xkb_new();
    return a->keymap;
}
static gpointer arch_get_egl_display(WPEDisplay *d, GError **e) {
    ArchDisplay *a = (ArchDisplay *)d;
    if (!a->drm) {
        /* Shared memory: Skia on the CPU, no EGL. */
        g_set_error_literal(e, WPE_EGL_ERROR, WPE_EGL_ERROR_NOT_AVAILABLE, "shared-memory frames: no EGL");
        return NULL;
    }
    if (!a->gbm) {
        a->gbm_fd = open(wpe_drm_device_get_render_node(a->drm), O_RDWR | O_CLOEXEC);
        if (a->gbm_fd >= 0) a->gbm = gbm_create_device(a->gbm_fd);
    }
    if (!a->gbm) {
        g_set_error_literal(e, WPE_EGL_ERROR, WPE_EGL_ERROR_NOT_AVAILABLE, "no GBM device on the render node");
        return NULL;
    }
    return eglGetPlatformDisplayEXT(EGL_PLATFORM_GBM_KHR, a->gbm, NULL);
}
static WPEInputMethodContext *arch_create_ime(WPEDisplay *d, WPEView *v) {
    ArchWeb *web = ((ArchView *)v)->web;
    ArchIme *ime = g_object_new(arch_ime_get_type(), "display", d, NULL);
    ime->web = web;
    if (web) web->ime = WPE_INPUT_METHOD_CONTEXT(ime);
    return WPE_INPUT_METHOD_CONTEXT(ime);
}
static void arch_display_class_init(ArchDisplayClass *k) {
    WPEDisplayClass *c = WPE_DISPLAY_CLASS(k);
    c->connect = arch_connect;
    c->create_view = arch_create_view;
    c->create_toplevel = arch_create_toplevel;
    c->get_drm_device = arch_get_drm;
    c->get_keymap = arch_get_keymap;
    c->get_egl_display = arch_get_egl_display;
    c->create_input_method_context = arch_create_ime;
}
static void arch_display_init(ArchDisplay *d) { d->gbm_fd = -1; }

/* One display for the process: WebKit asks the primary one for its device. */
static WPEDisplay *shared_display(const char *primary, const char *render, char **error) {
    static WPEDisplay *display;
    if (display) return display;
    ArchDisplay *a = g_object_new(arch_display_get_type(), NULL);
    if (render && primary) a->drm = wpe_drm_device_new(primary, render);
    GError *e = NULL;
    if (!wpe_display_connect(WPE_DISPLAY(a), &e)) {
        if (error) *error = g_strdup(e ? e->message : "the WPE display did not connect");
        g_clear_error(&e);
        g_object_unref(a);
        return NULL;
    }
    wpe_display_set_primary(WPE_DISPLAY(a));
    display = WPE_DISPLAY(a);
    return display;
}

/* ── its package, over the custom scheme ──────────────────────────────── */
/* The protocol handler is the process's; each request names its view. */
static ArchWeb *web_of_request(WebKitURISchemeRequest *req) {
    WebKitWebView *v = webkit_uri_scheme_request_get_web_view(req);
    return v ? g_object_get_data(G_OBJECT(v), "arch-web") : NULL;
}
static void on_scheme(WebKitURISchemeRequest *req, gpointer data) {
    ArchWeb *web = web_of_request(req);
    ArchResponse r = { 0 };
    const char *uri = webkit_uri_scheme_request_get_uri(req);
    if (!web || !web->cb.scheme || !web->cb.scheme(web->cb.user, uri, &r)) {
        GError *e = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "not in the package");
        webkit_uri_scheme_request_finish_error(req, e);
        g_error_free(e);
        return;
    }
    GBytes *bytes = g_bytes_new(r.body, r.len);
    GInputStream *stream = g_memory_input_stream_new_from_bytes(bytes);
    WebKitURISchemeResponse *res = webkit_uri_scheme_response_new(stream, (gint64)r.len);
    webkit_uri_scheme_response_set_status(res, (guint)r.status, NULL);
    if (r.mime) webkit_uri_scheme_response_set_content_type(res, r.mime);
    if (r.headers) {
        SoupMessageHeaders *h = soup_message_headers_new(SOUP_MESSAGE_HEADERS_RESPONSE);
        gchar **parts = g_strsplit(r.headers, "\n", -1);
        for (int i = 0; parts[i] && parts[i + 1]; i += 2) soup_message_headers_append(h, parts[i], parts[i + 1]);
        g_strfreev(parts);
        webkit_uri_scheme_response_set_http_headers(res, h);
    }
    webkit_uri_scheme_request_finish_with_response(req, res);
    g_object_unref(res);
    g_object_unref(stream);
    g_bytes_unref(bytes);
}

/* Only its package: no navigation elsewhere, no new windows. */
static gboolean on_policy(WebKitWebView *w, WebKitPolicyDecision *d, WebKitPolicyDecisionType type, gpointer data) {
    ArchWeb *web = data;
    if (type == WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION) {
        webkit_policy_decision_ignore(d);
        return TRUE;
    }
    if (type == WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION) {
        WebKitNavigationAction *a = webkit_navigation_policy_decision_get_navigation_action(WEBKIT_NAVIGATION_POLICY_DECISION(d));
        const char *uri = webkit_uri_request_get_uri(webkit_navigation_action_get_request(a));
        gsize n = strlen(web->scheme);
        if (!(g_str_has_prefix(uri, web->scheme) && uri[n] == ':') && strcmp(uri, "about:blank") != 0) {
            webkit_policy_decision_ignore(d);
            return TRUE;
        }
    }
    return FALSE;
}
static WebKitWebView *on_create(WebKitWebView *w, WebKitNavigationAction *a, gpointer data) { return NULL; }

static void on_message(WebKitUserContentManager *m, JSCValue *value, gpointer data) {
    ArchWeb *web = data;
    if (!web->cb.message || !jsc_value_is_string(value)) return;
    char *text = jsc_value_to_string(value);
    web->cb.message(web->cb.user, text);
    g_free(text);
}
static void on_load(WebKitWebView *w, WebKitLoadEvent ev, gpointer data) {
    ArchWeb *web = data;
    if (web->cb.loaded) web->cb.loaded(web->cb.user, (int)ev);
}
static void on_crash(WebKitWebView *w, WebKitWebProcessTerminationReason r, gpointer data) {
    ArchWeb *web = data;
    if (web->cb.crashed) web->cb.crashed(web->cb.user, (int)r);
}

ArchWeb *arch_web_new(const char *primary_node, const char *render_node, int width, int height, double scale,
                      const char *scheme, const char *boot_js, ArchCallbacks callbacks, char **error) {
    WPEDisplay *display = shared_display(primary_node, render_node, error);
    if (!display) return NULL;
    ArchWeb *web = g_new0(ArchWeb, 1);
    web->cb = callbacks;
    web->display = display;
    web->scheme = g_strdup(scheme);

    WebKitWebContext *context = webkit_web_context_get_default();
    static gboolean registered;
    /* One handler per scheme for the process: it finds the view by request. */
    if (!registered) {
        webkit_web_context_register_uri_scheme(context, scheme, on_scheme, NULL, NULL);
        WebKitSecurityManager *sec = webkit_web_context_get_security_manager(context);
        webkit_security_manager_register_uri_scheme_as_secure(sec, scheme);
        webkit_security_manager_register_uri_scheme_as_cors_enabled(sec, scheme);
        registered = TRUE;
    }

    WebKitUserContentManager *ucm = webkit_user_content_manager_new();
    webkit_user_content_manager_register_script_message_handler(ucm, "architect", NULL);
    g_signal_connect(ucm, "script-message-received::architect", G_CALLBACK(on_message), web);
    if (boot_js) {
        WebKitUserScript *s = webkit_user_script_new(boot_js, WEBKIT_USER_CONTENT_INJECT_TOP_FRAME,
                                                     WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START, NULL, NULL);
        webkit_user_content_manager_add_script(ucm, s);
        webkit_user_script_unref(s);
    }
    web->web = g_object_new(WEBKIT_TYPE_WEB_VIEW, "display", display, "user-content-manager", ucm, "web-context", context, NULL);
    g_object_unref(ucm);
    g_object_set_data(G_OBJECT(web->web), "arch-web", web);
    g_signal_connect(web->web, "decide-policy", G_CALLBACK(on_policy), web);
    g_signal_connect(web->web, "create", G_CALLBACK(on_create), web);
    g_signal_connect(web->web, "load-changed", G_CALLBACK(on_load), web);
    g_signal_connect(web->web, "web-process-terminated", G_CALLBACK(on_crash), web);

    web->view = webkit_web_view_get_wpe_view(web->web);
    ((ArchView *)web->view)->web = web;
    web->toplevel = wpe_view_get_toplevel(web->view);
    if (!web->toplevel) {
        web->toplevel = g_object_new(arch_toplevel_get_type(), "display", display, NULL);
        wpe_view_set_toplevel(web->view, web->toplevel);
    } else {
        g_object_ref(web->toplevel);
    }
    arch_web_resize(web, width, height, scale);
    wpe_view_set_visible(web->view, TRUE);
    wpe_view_map(web->view);
    return web;
}

void arch_web_free(ArchWeb *web) {
    if (!web) return;
    ((ArchView *)web->view)->web = NULL;
    g_clear_object(&web->pending);
    g_clear_object(&web->shown);
    g_clear_object(&web->toplevel);
    g_object_set_data(G_OBJECT(web->web), "arch-web", NULL);
    g_signal_handlers_disconnect_by_data(web->web, web);
    g_object_unref(web->web);
    g_free(web->scheme);
    g_free(web);
}
void arch_web_free_string(char *s) { g_free(s); }

void arch_web_load(ArchWeb *web, const char *uri) { webkit_web_view_load_uri(web->web, uri); }
void arch_web_reload(ArchWeb *web) { webkit_web_view_reload(web->web); }
void arch_web_resize(ArchWeb *web, int width, int height, double scale) {
    (void)scale;
    if (width < 1) width = 1;
    if (height < 1) height = 1;
    wpe_toplevel_resize(web->toplevel, width, height);
    wpe_view_resized(web->view, width, height);
}
void arch_web_eval(ArchWeb *web, const char *js) {
    webkit_web_view_evaluate_javascript(web->web, js, -1, NULL, NULL, NULL, NULL, NULL);
}
void arch_web_set_visible(ArchWeb *web, int visible) { wpe_view_set_visible(web->view, visible ? TRUE : FALSE); }

void arch_web_frame_done(ArchWeb *web, uint64_t id) {
    if (!web->pending || web->pending_id != id) return;
    if (web->shown) {
        wpe_view_buffer_released(web->view, web->shown);
        g_clear_object(&web->shown);
    }
    web->shown = web->pending;
    web->pending = NULL;
    wpe_view_buffer_rendered(web->view, web->shown);
}

static WPEModifiers modifiers(unsigned m) {
    WPEModifiers r = 0;
    if (m & 1) r |= WPE_MODIFIER_KEYBOARD_CONTROL;
    if (m & 2) r |= WPE_MODIFIER_KEYBOARD_SHIFT;
    if (m & 4) r |= WPE_MODIFIER_KEYBOARD_ALT;
    if (m & 8) r |= WPE_MODIFIER_KEYBOARD_META;
    return r;
}
static guint32 now_ms(void) { return (guint32)(g_get_monotonic_time() / 1000); }

void arch_web_pointer(ArchWeb *web, int kind, double x, double y, unsigned button, unsigned mods, unsigned press_count, int touch) {
    WPEInputSource src = touch ? WPE_INPUT_SOURCE_TOUCHSCREEN : WPE_INPUT_SOURCE_MOUSE;
    WPEEvent *ev = NULL;
    switch (kind) {
    case ARCH_POINTER_DOWN:
        ev = wpe_event_pointer_button_new(WPE_EVENT_POINTER_DOWN, web->view, src, now_ms(), modifiers(mods), button, x, y, press_count ? press_count : 1);
        break;
    case ARCH_POINTER_UP:
        ev = wpe_event_pointer_button_new(WPE_EVENT_POINTER_UP, web->view, src, now_ms(), modifiers(mods), button, x, y, 0);
        break;
    case ARCH_POINTER_MOVE:
        ev = wpe_event_pointer_move_new(WPE_EVENT_POINTER_MOVE, web->view, src, now_ms(), modifiers(mods), x, y, 0, 0);
        break;
    case ARCH_POINTER_ENTER:
        ev = wpe_event_pointer_move_new(WPE_EVENT_POINTER_ENTER, web->view, src, now_ms(), modifiers(mods), x, y, 0, 0);
        break;
    case ARCH_POINTER_LEAVE:
        ev = wpe_event_pointer_move_new(WPE_EVENT_POINTER_LEAVE, web->view, src, now_ms(), modifiers(mods), x, y, 0, 0);
        break;
    }
    if (ev) {
        wpe_view_event(web->view, ev);
        wpe_event_unref(ev);
    }
}
void arch_web_scroll(ArchWeb *web, double x, double y, double dx, double dy, int precise, unsigned mods) {
    WPEEvent *ev = wpe_event_scroll_new(web->view, WPE_INPUT_SOURCE_MOUSE, now_ms(), modifiers(mods), dx, dy, precise ? TRUE : FALSE, FALSE, x, y);
    wpe_view_event(web->view, ev);
    wpe_event_unref(ev);
}
void arch_web_key(ArchWeb *web, int down, unsigned keyval, unsigned keycode, unsigned mods) {
    WPEEvent *ev = wpe_event_keyboard_new(down ? WPE_EVENT_KEYBOARD_KEY_DOWN : WPE_EVENT_KEYBOARD_KEY_UP, web->view,
                                          WPE_INPUT_SOURCE_KEYBOARD, now_ms(), modifiers(mods), keycode, keyval);
    wpe_view_event(web->view, ev);
    wpe_event_unref(ev);
}
void arch_web_focus(ArchWeb *web, int focused) {
    if (focused) wpe_view_focus_in(web->view);
    else wpe_view_focus_out(web->view);
}
void arch_web_ime_commit(ArchWeb *web, const char *text) {
    if (web->ime) g_signal_emit_by_name(web->ime, "committed", text);
}
void arch_web_ime_preedit(ArchWeb *web, const char *text, unsigned cursor) {
    if (!web->ime) return;
    ArchIme *i = (ArchIme *)web->ime;
    g_free(i->preedit);
    i->preedit = g_strdup(text);
    i->cursor = cursor;
    g_signal_emit_by_name(web->ime, *text ? "preedit-changed" : "preedit-finished");
}

int arch_web_iterate(void) {
    int any = 0;
    while (g_main_context_iteration(NULL, FALSE)) any = 1;
    return any;
}
