/* Injects the quick control panel into the current YouTube TV WebKit page.
 * This keeps Cog's DRM fullscreen surface alive, so the panel is a real
 * picture overlay and the video never has to be stopped. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef void *gpointer;
typedef unsigned int guint;
typedef const struct OpaqueJSContext *JSContextRef;
typedef struct OpaqueJSString *JSStringRef;
typedef const struct OpaqueJSValue *JSValueRef;
typedef struct OpaqueJSValue *JSObjectRef;
typedef struct _WebKitWebExtension WebKitWebExtension;
typedef struct _WebKitWebPage WebKitWebPage;
typedef struct _WebKitFrame WebKitFrame;

extern guint g_signal_connect_data(gpointer, const char *, gpointer, gpointer, gpointer, int);
extern const char *webkit_web_page_get_uri(WebKitWebPage *);
extern WebKitFrame *webkit_web_page_get_main_frame(WebKitWebPage *);
extern JSContextRef webkit_frame_get_js_context(WebKitFrame *);
extern JSStringRef JSStringCreateWithUTF8CString(const char *);
extern JSValueRef JSEvaluateScript(JSContextRef, JSStringRef, JSObjectRef, JSStringRef, int, JSValueRef *);
extern void JSStringRelease(JSStringRef);

static void on_document_loaded(WebKitWebPage *page, gpointer data) {
    (void)data;
    const char *uri = webkit_web_page_get_uri(page);
    if (!uri || (strstr(uri, "youtube.com") == NULL && strstr(uri, "youtube-nocookie.com") == NULL)) return;
    WebKitFrame *frame = webkit_web_page_get_main_frame(page);
    JSContextRef context = frame ? webkit_frame_get_js_context(frame) : NULL;
    if (!context) return;

    FILE *f = fopen("/usr/lib/tv-control/control.js", "rb");
    if (!f) return;
    if (fseek(f, 0, SEEK_END) != 0) { fclose(f); return; }
    long n = ftell(f);
    if (n <= 0 || n > 65536 || fseek(f, 0, SEEK_SET) != 0) { fclose(f); return; }
    char *source = (char *)malloc((size_t)n + 1);
    if (!source) { fclose(f); return; }
    size_t got = fread(source, 1, (size_t)n, f);
    fclose(f);
    source[got] = '\0';
    JSStringRef script = JSStringCreateWithUTF8CString(source);
    if (script) {
        JSEvaluateScript(context, script, NULL, NULL, 1, NULL);
        JSStringRelease(script);
    }
    free(source);
}

static void on_page_created(WebKitWebExtension *extension, WebKitWebPage *page, gpointer data) {
    (void)extension; (void)data;
    g_signal_connect_data(page, "document-loaded", (gpointer)on_document_loaded, NULL, NULL, 0);
}

__attribute__((visibility("default")))
void webkit_web_extension_initialize(WebKitWebExtension *extension) {
    g_signal_connect_data(extension, "page-created", (gpointer)on_page_created, NULL, NULL, 0);
}
