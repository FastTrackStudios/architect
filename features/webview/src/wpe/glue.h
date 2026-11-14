/* The WPE WebKit glue (architect-webview, feature `wpe`): one web view
 * rendered off-screen by WPE WebKit through the WPE Platform API, its frames
 * handed to the app as dmabufs or shared memory, its input pushed in by the
 * app. features/webview/src/wpe.rs is the Rust side; glue.c the C side.
 * Everything here runs on the thread that called arch_web_new and iterates
 * the default GLib main context (arch_web_iterate). */
#pragma once
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ArchWeb ArchWeb;

/* One frame. `kind` 1: dmabuf (fd, fourcc, modifier, offset, stride of
 * plane 0; n_planes), 2: shm (pixels, stride; BGRA premultiplied,
 * fourcc ARGB8888/XRGB8888). The frame stays valid until
 * arch_web_frame_done with its `id`. */
typedef struct {
    uint64_t id;
    int kind;
    int width, height;
    uint32_t fourcc;
    uint64_t modifier;
    int n_planes;
    int fds[4];
    uint32_t offsets[4];
    uint32_t strides[4];
    const uint8_t *pixels;
    size_t size;
} ArchFrame;

/* What the protocol handler answers (`architect-ext://…`): its bytes are
 * copied. */
typedef struct {
    int status;
    const char *mime;
    const uint8_t *body;
    size_t len;
    /* "name\nvalue\nname\nvalue…" */
    const char *headers;
} ArchResponse;

typedef struct {
    void *user;
    void (*frame)(void *user, const ArchFrame *frame);
    void (*message)(void *user, const char *json);
    /* Fill `out`; return nonzero when answered. */
    int (*scheme)(void *user, const char *uri, ArchResponse *out);
    void (*cursor)(void *user, const char *name);
    void (*ime_area)(void *user, int x, int y, int w, int h);
    void (*loaded)(void *user, int event);
    void (*crashed)(void *user, int reason);
} ArchCallbacks;

/* A view of `width`×`height` logical pixels at `scale`. `render_node`
 * ("/dev/dri/renderD128") for dmabuf frames; NULL for shared memory (CPU
 * rendering). `boot_js` runs at document start in its top frame. NULL when
 * WPE could not start (the reason in `error`, which the caller frees with
 * arch_web_free_string). */
ArchWeb *arch_web_new(const char *primary_node, const char *render_node, int width, int height, double scale,
                      const char *scheme, const char *boot_js, ArchCallbacks callbacks, char **error);
void arch_web_free(ArchWeb *web);
void arch_web_free_string(char *s);

void arch_web_load(ArchWeb *web, const char *uri);
void arch_web_reload(ArchWeb *web);
void arch_web_resize(ArchWeb *web, int width, int height, double scale);
void arch_web_eval(ArchWeb *web, const char *js);
void arch_web_frame_done(ArchWeb *web, uint64_t id);
void arch_web_set_visible(ArchWeb *web, int visible);

/* Input, in the view's logical pixels. `mods`: 1 control, 2 shift, 4 alt,
 * 8 meta. */
enum { ARCH_POINTER_DOWN = 1, ARCH_POINTER_UP = 2, ARCH_POINTER_MOVE = 3, ARCH_POINTER_ENTER = 4, ARCH_POINTER_LEAVE = 5 };
void arch_web_pointer(ArchWeb *web, int kind, double x, double y, unsigned button, unsigned mods, unsigned press_count, int touch);
void arch_web_scroll(ArchWeb *web, double x, double y, double dx, double dy, int precise, unsigned mods);
void arch_web_key(ArchWeb *web, int down, unsigned keyval, unsigned keycode, unsigned mods);
void arch_web_focus(ArchWeb *web, int focused);
void arch_web_ime_commit(ArchWeb *web, const char *text);
void arch_web_ime_preedit(ArchWeb *web, const char *text, unsigned cursor);

/* Runs what is due on the default main context, without waiting: whether
 * anything ran. */
int arch_web_iterate(void);

#ifdef __cplusplus
}
#endif
