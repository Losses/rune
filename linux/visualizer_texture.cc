#include "visualizer_texture.h"

#include <dlfcn.h>
#include <string.h>

#include <cstdint>

namespace {

// Log the first few occurrences of an event, then every 300th.
bool should_log(guint64 count) { return count <= 3 || count % 300 == 0; }

// The C ABI exported by the Rust hub (`native/hub/src/visualizer.rs`).
struct RuneVisualizerFrame {
  const uint8_t* pixels;  // height rows of |stride| bytes, RGBA
  uint32_t width;
  uint32_t height;
  size_t stride;
  uint64_t id;
};
using FrameInfoFunction = uint64_t (*)(uint32_t*, uint32_t*);
using AcquireFrameFunction = const void* (*)(RuneVisualizerFrame*);
using ReleaseFrameFunction = void (*)(const void*);

struct HubApi {
  FrameInfoFunction frame_info = nullptr;
  AcquireFrameFunction acquire_frame = nullptr;
  ReleaseFrameFunction release_frame = nullptr;
};

// Resolves the hub's symbols once. The Dart side (rinf) loads libhub.so
// itself, so it is normally in the process already (RTLD_NOLOAD); otherwise
// the dynamic loader finds it through the executable's rpath. Resolving at run
// time avoids a link-time dependency and tolerates missing symbols.
const HubApi* get_hub_api() {
  static const HubApi* api = [] {
    void* handle = dlopen("libhub.so", RTLD_NOW | RTLD_NOLOAD);
    if (handle == nullptr) {
      handle = dlopen("libhub.so", RTLD_NOW);
    }
    HubApi* hub = new HubApi();
    if (handle != nullptr) {
      hub->frame_info = reinterpret_cast<FrameInfoFunction>(
          dlsym(handle, "rune_visualizer_frame_info"));
      hub->acquire_frame = reinterpret_cast<AcquireFrameFunction>(
          dlsym(handle, "rune_visualizer_acquire_frame"));
      hub->release_frame = reinterpret_cast<ReleaseFrameFunction>(
          dlsym(handle, "rune_visualizer_release_frame"));
    }
    g_message(
        "visualizer: libhub.so=%d symbols: frame_info=%d acquire_frame=%d "
        "release_frame=%d",
        handle != nullptr, hub->frame_info != nullptr,
        hub->acquire_frame != nullptr, hub->release_frame != nullptr);
    return hub;
  }();
  return api;
}

bool hub_api_complete(const HubApi* api) {
  return api->frame_info && api->acquire_frame && api->release_frame;
}

}  // namespace

// A Flutter texture fed by the Rust visualizer. The engine calls copy_pixels
// on its raster thread, with the GL context current, and uploads the returned
// pixels with glTexImage2D(GL_RGBA) straight afterwards. The texture holds a
// hub frame between two calls, so only the raster thread touches its fields;
// dispose runs after the last reference, when no call can be in flight.
G_DECLARE_FINAL_TYPE(VisualizerTexture, visualizer_texture, VISUALIZER, TEXTURE,
                     FlPixelBufferTexture)

struct _VisualizerTexture {
  FlPixelBufferTexture parent_instance;

  // The hub frame handed to the engine by the last call, or null.
  const void* frame;
  // Tight copy of a frame whose rows are padded, and its size in bytes.
  uint8_t* copy;
  size_t copy_size;
  guint64 handed_over;
  guint64 copies;
};

G_DEFINE_TYPE(VisualizerTexture,
              visualizer_texture,
              fl_pixel_buffer_texture_get_type())

// Gives the held frame back to the renderer.
static void visualizer_texture_release_frame(VisualizerTexture* self) {
  if (self->frame != nullptr) {
    get_hub_api()->release_frame(self->frame);
    self->frame = nullptr;
  }
}

// Implements FlPixelBufferTexture::copy_pixels.
static gboolean visualizer_texture_copy_pixels(FlPixelBufferTexture* texture,
                                               const uint8_t** buffer,
                                               uint32_t* width,
                                               uint32_t* height,
                                               GError** error) {
  VisualizerTexture* self = VISUALIZER_TEXTURE(texture);

  // The previous frame has been uploaded by now.
  visualizer_texture_release_frame(self);

  RuneVisualizerFrame frame = {};
  const void* handle = get_hub_api()->acquire_frame(&frame);
  if (handle == nullptr) {
    g_set_error(error, g_quark_from_static_string("visualizer"), 0,
                "no frame yet");
    return FALSE;
  }

  const size_t row = static_cast<size_t>(frame.width) * 4;
  if (frame.stride == row) {
    // Tight rows: the engine reads the hub's pixels directly, and the frame
    // stays held until the next call or dispose.
    self->frame = handle;
    *buffer = frame.pixels;
    if (should_log(++self->handed_over)) {
      g_message("visualizer: copy_pixels #%" G_GUINT64_FORMAT
                ": frame %" G_GUINT64_FORMAT
                " handed over without a copy (%ux%u)",
                self->handed_over, static_cast<guint64>(frame.id), frame.width,
                frame.height);
    }
  } else {
    // Padded rows: the engine reads tightly packed RGBA, so copy and give the
    // renderer its buffer back at once.
    const size_t size = row * frame.height;
    if (self->copy_size != size) {
      g_free(self->copy);
      self->copy = static_cast<uint8_t*>(g_malloc(size));
      self->copy_size = size;
    }
    for (uint32_t y = 0; y < frame.height; ++y) {
      memcpy(self->copy + y * row, frame.pixels + y * frame.stride, row);
    }
    get_hub_api()->release_frame(handle);
    *buffer = self->copy;
    if (should_log(++self->copies)) {
      g_message("visualizer: copy_pixels #%" G_GUINT64_FORMAT
                ": frame %" G_GUINT64_FORMAT " copied (%ux%u, stride %zu)",
                self->copies, static_cast<guint64>(frame.id), frame.width,
                frame.height, frame.stride);
    }
  }
  *width = frame.width;
  *height = frame.height;
  return TRUE;
}

// Implements GObject::dispose.
static void visualizer_texture_dispose(GObject* object) {
  VisualizerTexture* self = VISUALIZER_TEXTURE(object);
  visualizer_texture_release_frame(self);
  g_clear_pointer(&self->copy, g_free);
  self->copy_size = 0;
  G_OBJECT_CLASS(visualizer_texture_parent_class)->dispose(object);
}

static void visualizer_texture_class_init(VisualizerTextureClass* klass) {
  G_OBJECT_CLASS(klass)->dispose = visualizer_texture_dispose;
  FL_PIXEL_BUFFER_TEXTURE_CLASS(klass)->copy_pixels =
      visualizer_texture_copy_pixels;
}

static void visualizer_texture_init(VisualizerTexture* self) {}

struct _VisualizerChannel {
  FlTextureRegistrar* registrar;
  FlMethodChannel* channel;
  VisualizerTexture* texture;  // owned, null if there is none
  guint refresh_source;        // GSource id of the poll, 0 if none
  guint64 announced_frame_id;
  guint64 announcements;
};

// Polls the hub for new frames and tells the engine about them. Runs on the
// main thread, which is where texture registration happens too.
static gboolean visualizer_channel_refresh(gpointer user_data) {
  VisualizerChannel* self = static_cast<VisualizerChannel*>(user_data);
  guint64 frame_id = get_hub_api()->frame_info(nullptr, nullptr);
  if (frame_id != 0 && frame_id != self->announced_frame_id) {
    self->announced_frame_id = frame_id;
    if (should_log(++self->announcements)) {
      g_message("visualizer: frame available #%" G_GUINT64_FORMAT
                ": frame %" G_GUINT64_FORMAT " texture %" G_GINT64_FORMAT,
                self->announcements, frame_id,
                fl_texture_get_id(FL_TEXTURE(self->texture)));
    }
    fl_texture_registrar_mark_texture_frame_available(
        self->registrar, FL_TEXTURE(self->texture));
  }
  return G_SOURCE_CONTINUE;
}

// Stops refreshing and unregisters the texture, if there is one.
static void visualizer_channel_dispose_texture(VisualizerChannel* self) {
  if (self->texture == nullptr) {
    return;
  }
  g_message("visualizer: dispose texture %" G_GINT64_FORMAT
            " after %" G_GUINT64_FORMAT " frames announced",
            fl_texture_get_id(FL_TEXTURE(self->texture)), self->announcements);
  if (self->refresh_source != 0) {
    g_source_remove(self->refresh_source);
    self->refresh_source = 0;
  }
  fl_texture_registrar_unregister_texture(self->registrar,
                                          FL_TEXTURE(self->texture));
  g_clear_object(&self->texture);
}

static FlMethodResponse* visualizer_channel_create(VisualizerChannel* self) {
  visualizer_channel_dispose_texture(self);

  const HubApi* api = get_hub_api();
  if (!hub_api_complete(api)) {
    g_warning("visualizer: create failed: symbols missing from libhub.so");
    return FL_METHOD_RESPONSE(fl_method_error_response_new(
        "unavailable", "The visualizer symbols are missing from libhub.so.",
        nullptr));
  }

  self->texture = VISUALIZER_TEXTURE(
      g_object_new(visualizer_texture_get_type(), nullptr));
  if (!fl_texture_registrar_register_texture(self->registrar,
                                             FL_TEXTURE(self->texture))) {
    g_clear_object(&self->texture);
    g_warning("visualizer: create failed: texture registration refused");
    return FL_METHOD_RESPONSE(fl_method_error_response_new(
        "unavailable", "The engine refused the texture.", nullptr));
  }
  int64_t id = fl_texture_get_id(FL_TEXTURE(self->texture));
  g_message("visualizer: registered texture %" G_GINT64_FORMAT, id);

  self->announced_frame_id = 0;
  self->announcements = 0;
  // Linux has no display link for the embedder; poll a little faster than 120 Hz.
  self->refresh_source = g_timeout_add(8, visualizer_channel_refresh, self);

  g_autoptr(FlValue) result = fl_value_new_int(id);
  return FL_METHOD_RESPONSE(fl_method_success_response_new(result));
}

static void visualizer_channel_method_call(FlMethodChannel* channel,
                                           FlMethodCall* method_call,
                                           gpointer user_data) {
  VisualizerChannel* self = static_cast<VisualizerChannel*>(user_data);
  const gchar* method = fl_method_call_get_name(method_call);

  g_autoptr(FlMethodResponse) response = nullptr;
  if (strcmp(method, "create") == 0) {
    response = visualizer_channel_create(self);
  } else if (strcmp(method, "dispose") == 0) {
    visualizer_channel_dispose_texture(self);
    response = FL_METHOD_RESPONSE(fl_method_success_response_new(nullptr));
  } else {
    response = FL_METHOD_RESPONSE(fl_method_not_implemented_response_new());
  }

  g_autoptr(GError) error = nullptr;
  if (!fl_method_call_respond(method_call, response, &error)) {
    g_warning("visualizer: failed to send response: %s", error->message);
  }
}

VisualizerChannel* visualizer_channel_new(FlEngine* engine) {
  VisualizerChannel* self = g_new0(VisualizerChannel, 1);
  self->registrar =
      FL_TEXTURE_REGISTRAR(g_object_ref(fl_engine_get_texture_registrar(engine)));
  self->channel = fl_method_channel_new(
      fl_engine_get_binary_messenger(engine), "not.ci.rune/visualizer",
      FL_METHOD_CODEC(fl_standard_method_codec_new()));
  fl_method_channel_set_method_call_handler(
      self->channel, visualizer_channel_method_call, self, nullptr);
  g_message("visualizer: channel registered");
  return self;
}

void visualizer_channel_free(VisualizerChannel* self) {
  if (self == nullptr) {
    return;
  }
  // No call can arrive once the handler is gone.
  fl_method_channel_set_method_call_handler(self->channel, nullptr, nullptr,
                                            nullptr);
  visualizer_channel_dispose_texture(self);
  g_clear_object(&self->channel);
  g_clear_object(&self->registrar);
  g_free(self);
}
