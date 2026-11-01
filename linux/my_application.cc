#include "my_application.h"

#include <dlfcn.h>
#include <flutter_linux/flutter_linux.h>
#include <bitsdojo_window_linux/bitsdojo_window_plugin.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include "flutter/generated_plugin_registrant.h"

// Global reference to the bitsdojo window for dynamic frame control
static GtkWindow* main_window = nullptr;
static void* g_bdw = nullptr;

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;

  // Delivers scene files (.runescene) to Dart.
  FlMethodChannel* scene_channel;
  // Scene file paths received before Dart called takePending.
  GPtrArray* pending_scene_files;
  gboolean dart_listening;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// Simple method channel handler for Linux window frame control
static void method_call_handler(FlMethodChannel* channel, FlMethodCall* method_call,
                                gpointer user_data) {
  const gchar* method = fl_method_call_get_name(method_call);

  if (strcmp(method, "set_custom_frame") == 0) {
    g_print("set_custom_frame method called\n");

    FlValue* args = fl_method_call_get_args(method_call);
    if (fl_value_get_type(args) == FL_VALUE_TYPE_MAP) {
      FlValue* enabled_value = fl_value_lookup_string(args, "enabled");
      if (enabled_value != nullptr) {
        bool enabled = fl_value_get_bool(enabled_value);
        g_print("Custom frame setting: %s\n", enabled ? "enabled" : "disabled");

        // Actually implement the frame control
        if (main_window != nullptr) {
          if (enabled) {
            // Enable custom frame: remove GTK decorations for frameless window
            gtk_window_set_decorated(main_window, FALSE);
            g_print("GTK window decorations removed (frameless mode)\n");
          } else {
            // Disable custom frame: restore GTK decorations
            gtk_window_set_decorated(main_window, TRUE);
            g_print("GTK window decorations restored (framed mode)\n");
          }
        } else {
          g_print("Error: main_window is null\n");
        }
      }
    }

    // Return success response
    g_autoptr(FlValue) result = fl_value_new_bool(true);
    fl_method_call_respond(method_call, FL_METHOD_RESPONSE(fl_method_success_response_new(result)), nullptr);
  } else {
    fl_method_call_respond_not_implemented(method_call, nullptr);
  }
}

// Scene files: paths queue until Dart has called takePending once, then they
// are sent with "open".
static void scene_method_call_handler(FlMethodChannel* channel,
                                      FlMethodCall* method_call,
                                      gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  if (strcmp(fl_method_call_get_name(method_call), "takePending") == 0) {
    g_autoptr(FlValue) paths = fl_value_new_list();
    for (guint i = 0; i < self->pending_scene_files->len; i++) {
      fl_value_append_take(
          paths, fl_value_new_string(
                     static_cast<const gchar*>(self->pending_scene_files->pdata[i])));
    }
    g_ptr_array_set_size(self->pending_scene_files, 0);
    self->dart_listening = TRUE;
    g_autoptr(FlMethodResponse) response =
        FL_METHOD_RESPONSE(fl_method_success_response_new(paths));
    fl_method_call_respond(method_call, response, nullptr);
  } else {
    fl_method_call_respond_not_implemented(method_call, nullptr);
  }
}

// Queues the path, or sends it to Dart once Dart is listening.
static void receive_scene_file(MyApplication* self, const gchar* path) {
  if (self->dart_listening && self->scene_channel != nullptr) {
    g_autoptr(FlValue) paths = fl_value_new_list();
    fl_value_append_take(paths, fl_value_new_string(path));
    fl_method_channel_invoke_method(self->scene_channel, "open", paths, nullptr,
                                    nullptr, nullptr);
  } else {
    g_ptr_array_add(self->pending_scene_files, g_strdup(path));
  }
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);

  // A second launch activates this instance: show the window it already has.
  if (main_window != nullptr) {
    gtk_window_present(main_window);
    return;
  }

  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));

  // Enforce client side decorations iff. GTK_CSD is present and equal to 1
  if (g_strcmp0(g_getenv("GTK_CSD"), "1") == 0) {
    GtkHeaderBar* header_bar = GTK_HEADER_BAR(gtk_header_bar_new());
    gtk_widget_show(GTK_WIDGET(header_bar));
    gtk_header_bar_set_title(header_bar, "rune");
    gtk_header_bar_set_show_close_button(header_bar, TRUE);
    gtk_window_set_titlebar(window, GTK_WIDGET(header_bar));
  } else {
    gtk_window_set_title(window, "rune");
  }

  // Initialize bitsdojo window and set initial frame
  g_bdw = bitsdojo_window_from(window);
  // Note: We can't easily cast this to the correct type, so we'll handle it differently
  // For now, we initialize with the default frame setting

  gtk_window_set_default_size(window, 0, 0);
  gtk_widget_show(GTK_WIDGET(window));

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  // Use Skia to avoid large transient render-target retirement during popups.
  fl_dart_project_set_enable_impeller(project, FALSE);
  fl_dart_project_set_dart_entrypoint_arguments(project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  // Store references for method channel access
  main_window = window;

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));

  // The Rust hub registers the visualizer's texture with the engine itself. Its
  // symbols are looked up at run time: the Dart side (rinf) loads libhub.so as
  // well, and a missing library or symbol only leaves the visualizer without a
  // texture.
  void* hub = dlopen("libhub.so", RTLD_NOW | RTLD_NOLOAD);
  if (hub == nullptr) {
    hub = dlopen("libhub.so", RTLD_NOW);
  }
  if (hub != nullptr) {
    if (auto attach = reinterpret_cast<bool (*)(void*)>(
            dlsym(hub, "rune_flutter_attach"))) {
      attach(fl_view_get_engine(view));
    }
  }

  // Initialize bitsdojo window
  g_bdw = bitsdojo_window_from(window);

  // Set up simple method channel for Linux window frame control
  g_autoptr(FlMethodChannel) channel = fl_method_channel_new(
      fl_engine_get_binary_messenger(fl_view_get_engine(view)),
      "not.ci.rune/linux_window_frame",
      FL_METHOD_CODEC(fl_standard_method_codec_new()));

  fl_method_channel_set_method_call_handler(channel, method_call_handler, nullptr, nullptr);

  self->scene_channel = fl_method_channel_new(
      fl_engine_get_binary_messenger(fl_view_get_engine(view)),
      "ci.not.rune/scene_files",
      FL_METHOD_CODEC(fl_standard_method_codec_new()));
  fl_method_channel_set_method_call_handler(self->scene_channel,
                                            scene_method_call_handler, self,
                                            nullptr);

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

// Implements GApplication::open. A second launch forwards its scene files to
// this instance, which is the only one with a window.
static void my_application_open(GApplication* application, GFile** files,
                                gint n_files, const gchar* hint) {
  MyApplication* self = MY_APPLICATION(application);
  for (gint i = 0; i < n_files; i++) {
    g_autofree gchar* path = g_file_get_path(files[i]);
    if (path != nullptr) {
      receive_scene_file(self, path);
    }
  }
  if (main_window != nullptr) {
    gtk_window_present(main_window);
  }
}

// Implements GApplication::local_command_line.
static gboolean my_application_local_command_line(GApplication* application, gchar*** arguments, int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  // Strip out the first argument as it is the binary name.
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);

  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
     g_warning("Failed to register: %s", error->message);
     *exit_status = 1;
     return TRUE;
  }

  if (g_application_get_is_remote(application)) {
    // Another instance is running. Hand it the scene files, or just ask it to
    // show its window, and let this process exit.
    g_autoptr(GPtrArray) files = g_ptr_array_new_with_free_func(g_object_unref);
    for (gchar** argument = self->dart_entrypoint_arguments;
         *argument != nullptr; argument++) {
      g_autofree gchar* lower = g_ascii_strdown(*argument, -1);
      if (g_str_has_suffix(lower, ".runescene")) {
        g_ptr_array_add(files, g_file_new_for_commandline_arg(*argument));
      }
    }
    if (files->len > 0) {
      g_application_open(application, reinterpret_cast<GFile**>(files->pdata),
                         files->len, "");
    } else {
      g_application_activate(application);
    }
    *exit_status = 0;
    return TRUE;
  }

  g_application_activate(application);
  *exit_status = 0;

  return TRUE;
}

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  //MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application startup.

  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  //MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application shutdown.

  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

// Implements GObject::dispose.
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  g_clear_object(&self->scene_channel);
  g_clear_pointer(&self->pending_scene_files, g_ptr_array_unref);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->open = my_application_open;
  G_APPLICATION_CLASS(klass)->local_command_line = my_application_local_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {
  self->pending_scene_files = g_ptr_array_new_with_free_func(g_free);
}

MyApplication* my_application_new() {
  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID,
                                     "flags", G_APPLICATION_HANDLES_OPEN,
                                     nullptr));
}
