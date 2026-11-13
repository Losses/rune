#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <strings.h>

#include "flutter-pi.h"
#include "pluginregistry.h"
#include "platformchannel.h"

struct BDWAPI {
    intptr_t (*getAppWindowHandle)(void);
    void (*getScreenRect)(intptr_t, int32_t*, int32_t*, int32_t*, int32_t*);
    void (*getScaleFactor)(intptr_t, int32_t*);
    void (*getPosition)(intptr_t, int32_t*, int32_t*);
    void (*setPosition)(intptr_t, int32_t, int32_t);
    void (*getSize)(intptr_t, int32_t*, int32_t*);
    void (*setSize)(intptr_t, int32_t, int32_t);
    void (*setRect)(intptr_t, int32_t, int32_t, int32_t, int32_t);
    void (*setMinSize)(intptr_t, int32_t, int32_t);
    void (*setMaxSize)(intptr_t, int32_t, int32_t);
    void (*showWindow)(intptr_t);
    void (*hideWindow)(intptr_t);
    void (*minimizeWindow)(intptr_t);
    void (*maximizeWindow)(intptr_t);
    void (*unmaximizeWindow)(intptr_t);
    void (*setWindowTitle)(intptr_t, const char*);
};

static intptr_t bdw_getAppWindowHandle(void) { return 0; }
static void bdw_getScreenRect(intptr_t w, int32_t *x, int32_t *y, int32_t *width, int32_t *height) {
    if (x) *x = 0; if (y) *y = 0; if (width) *width = 1024; if (height) *height = 600;
}
static void bdw_getScaleFactor(intptr_t w, int32_t *scale) { if (scale) *scale = 1; }
static void bdw_getPosition(intptr_t w, int32_t *x, int32_t *y) { if (x) *x = 0; if (y) *y = 0; }
static void bdw_setPosition(intptr_t w, int32_t x, int32_t y) {}
static void bdw_getSize(intptr_t w, int32_t *width, int32_t *height) { if (width) *width = 1024; if (height) *height = 600; }
static void bdw_setSize(intptr_t w, int32_t width, int32_t height) {}
static void bdw_setRect(intptr_t w, int32_t x, int32_t y, int32_t width, int32_t height) {}
static void bdw_setMinSize(intptr_t w, int32_t width, int32_t height) {}
static void bdw_setMaxSize(intptr_t w, int32_t width, int32_t height) {}
static void bdw_showWindow(intptr_t w) {}
static void bdw_hideWindow(intptr_t w) {}
static void bdw_minimizeWindow(intptr_t w) {}
static void bdw_maximizeWindow(intptr_t w) {}
static void bdw_unmaximizeWindow(intptr_t w) {}
static void bdw_setWindowTitle(intptr_t w, const char *title) {}

static struct BDWAPI g_bdw_api = {
    .getAppWindowHandle = bdw_getAppWindowHandle,
    .getScreenRect = bdw_getScreenRect,
    .getScaleFactor = bdw_getScaleFactor,
    .getPosition = bdw_getPosition,
    .setPosition = bdw_setPosition,
    .getSize = bdw_getSize,
    .setSize = bdw_setSize,
    .setRect = bdw_setRect,
    .setMinSize = bdw_setMinSize,
    .setMaxSize = bdw_setMaxSize,
    .showWindow = bdw_showWindow,
    .hideWindow = bdw_hideWindow,
    .minimizeWindow = bdw_minimizeWindow,
    .maximizeWindow = bdw_maximizeWindow,
    .unmaximizeWindow = bdw_unmaximizeWindow,
    .setWindowTitle = bdw_setWindowTitle,
};

__attribute__((visibility("default"))) struct BDWAPI *bitsdojo_window_api(void) {
    return &g_bdw_api;
}

static void on_desktop_message(void *userdata, const FlutterPlatformMessage *message) {
    (void) userdata;
    if (message->response_handle == NULL) {
        return;
    }

    if (strncmp(message->channel, "flutter/system_tray/", 20) == 0) {
        platch_respond_success_std(message->response_handle, &STDBOOL(true));
        return;
    }

    struct platch_obj object;
    int ok = platch_decode((const uint8_t *) message->message, message->message_size, kStandardMethodCall, &object);
    if (ok != 0 || object.method == NULL) {
        platch_respond_success_std(message->response_handle, &STDNULL);
        return;
    }

    const char *m = object.method;

    if (strcasecmp(m, "getDirectoryPath") == 0 ||
        strcasecmp(m, "getDirectoryPaths") == 0) {
        const char *music_dir = getenv("HOME");
        char path_buf[512];
        if (music_dir) {
            snprintf(path_buf, sizeof(path_buf), "%s/Music", music_dir);
        } else {
            snprintf(path_buf, sizeof(path_buf), "/home/dietpi/Music");
        }
        struct std_value list = STDLIST1(STDSTRING(path_buf));
        platch_respond_success_std(message->response_handle, &list);
        return;
    }

    if (strcasecmp(m, "isMaximized") == 0 ||
        strcasecmp(m, "isVisible") == 0 ||
        strcasecmp(m, "isFocused") == 0 ||
        strcasecmp(m, "initSystemTray") == 0 ||
        strcasecmp(m, "init") == 0) {
        platch_respond_success_std(message->response_handle, &STDBOOL(true));
        return;
    }

    if (strcasecmp(m, "isFullScreen") == 0 ||
        strcasecmp(m, "isMinimized") == 0 ||
        strcasecmp(m, "isPreventClose") == 0) {
        platch_respond_success_std(message->response_handle, &STDBOOL(false));
        return;
    }

    if (strcasecmp(m, "getSize") == 0 ||
        strcasecmp(m, "getMinimumSize") == 0 ||
        strcasecmp(m, "getMaximumSize") == 0) {
        struct std_value map = STDMAP2(
            STDSTRING("width"), STDFLOAT64(1024.0),
            STDSTRING("height"), STDFLOAT64(600.0)
        );
        platch_respond_success_std(message->response_handle, &map);
        return;
    }

    if (strcasecmp(m, "getPosition") == 0 ||
        strcasecmp(m, "getCursorScreenPoint") == 0) {
        struct std_value map = STDMAP2(
            STDSTRING("x"), STDFLOAT64(0.0),
            STDSTRING("y"), STDFLOAT64(0.0)
        );
        platch_respond_success_std(message->response_handle, &map);
        return;
    }

    if (strcasecmp(m, "getPrimaryDisplay") == 0 ||
        strcasecmp(m, "getAllDisplays") == 0) {
        struct std_value size_map = STDMAP2(
            STDSTRING("width"), STDFLOAT64(1024.0),
            STDSTRING("height"), STDFLOAT64(600.0)
        );
        struct std_value pos_map = STDMAP2(
            STDSTRING("x"), STDFLOAT64(0.0),
            STDSTRING("y"), STDFLOAT64(0.0)
        );
        struct std_value display = STDMAP6(
            STDSTRING("id"), STDINT64(0),
            STDSTRING("name"), STDSTRING("Builtin"),
            STDSTRING("size"), size_map,
            STDSTRING("visiblePosition"), pos_map,
            STDSTRING("visibleSize"), size_map,
            STDSTRING("scaleFactor"), STDFLOAT64(1.0)
        );
        if (strcasecmp(m, "getAllDisplays") == 0) {
            struct std_value list = STDLIST1(display);
            platch_respond_success_std(message->response_handle, &list);
        } else {
            platch_respond_success_std(message->response_handle, &display);
        }
        return;
    }

    if (strcasecmp(m, "getTitle") == 0) {
        struct std_value title = STDSTRING("Rune");
        platch_respond_success_std(message->response_handle, &title);
        return;
    }

    platch_respond_success_std(message->response_handle, &STDNULL);
}

static const char *kDesktopChannels[] = {
    "window_manager",
    "screen_retriever",
    "flutter/system_tray/tray",
    "flutter/system_tray/menu_manager",
    "flutter/system_tray/app_window",
    "system_tray",
    "com.alexmercerind/flutter_acrylic",
    "flutter_acrylic",
    "local_notifier",
    "desktop_drop",
    "system_theme",
    "flutter_window_close",
    "flutter_window_close_notification",
    "plugins.flutter.dev/file_selector_linux",
    "plugins.flutter.io/file_selector",
    "plugins.flutter.io/url_launcher_linux",
    "plugins.flutter.io/url_launcher",
    "flutter/system_sound",
    "flutter/mousecursor",
    NULL
};

enum plugin_init_result desktop_stubs_init(struct flutterpi *flutterpi, void **userdata_out) {
    struct plugin_registry *registry = flutterpi_get_plugin_registry(flutterpi);
    for (int i = 0; kDesktopChannels[i] != NULL; i++) {
        plugin_registry_set_receiver_v2_locked(registry, kDesktopChannels[i], on_desktop_message, NULL);
    }
    *userdata_out = NULL;
    return PLUGIN_INIT_RESULT_INITIALIZED;
}

void desktop_stubs_deinit(struct flutterpi *flutterpi, void *userdata) {
    (void) userdata;
    struct plugin_registry *registry = flutterpi_get_plugin_registry(flutterpi);
    for (int i = 0; kDesktopChannels[i] != NULL; i++) {
        plugin_registry_remove_receiver_v2_locked(registry, kDesktopChannels[i]);
    }
}

FLUTTERPI_PLUGIN("desktop_stubs", desktop_stubs, desktop_stubs_init, desktop_stubs_deinit)
