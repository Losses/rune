#include "visualizer_texture.h"

#include <dlfcn.h>
#include <epoxy/egl.h>
#include <epoxy/gl.h>
#ifdef GDK_WINDOWING_WAYLAND
#include <gdk/gdkwayland.h>
#endif
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif
#include <string.h>
#include <unistd.h>

#include <atomic>
#include <cstdint>
#include <mutex>
#include <utility>
#include <vector>

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
struct RuneVisualizerSharedFrame {
  // The VK_FORMAT_R8G8B8A8_UNORM image (optimal tiling) the frame is in; never
  // reused for another image.
  uint64_t image_id;
  // Images of an earlier set are no longer drawn into.
  uint64_t image_set;
  // Size of the image's dedicated allocation.
  uint64_t memory_size;
  uint32_t width;
  uint32_t height;
  uint64_t id;
};
using FrameInfoFunction = uint64_t (*)(uint32_t*, uint32_t*);
using AcquireFrameFunction = const void* (*)(RuneVisualizerFrame*);
using ReleaseFrameFunction = void (*)(const void*);
using SetGraphicsDeviceFunction = void (*)(const uint8_t*, const uint8_t*);
using AcquireSharedFrameFunction = const void* (*)(RuneVisualizerSharedFrame*);
using ExportFrameFdFunction = int (*)(const void*);
using FallBackToReadbackFunction = void (*)();

struct HubApi {
  FrameInfoFunction frame_info = nullptr;
  AcquireFrameFunction acquire_frame = nullptr;
  ReleaseFrameFunction release_frame = nullptr;
  // Optional: without them frames are only read back.
  SetGraphicsDeviceFunction set_graphics_device = nullptr;
  AcquireSharedFrameFunction acquire_shared_frame = nullptr;
  ExportFrameFdFunction export_frame_memory = nullptr;
  ExportFrameFdFunction export_frame_semaphore = nullptr;
  FallBackToReadbackFunction fall_back_to_readback = nullptr;
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
      hub->set_graphics_device = reinterpret_cast<SetGraphicsDeviceFunction>(
          dlsym(handle, "rune_visualizer_set_graphics_device"));
      hub->acquire_shared_frame = reinterpret_cast<AcquireSharedFrameFunction>(
          dlsym(handle, "rune_visualizer_acquire_shared_frame"));
      hub->export_frame_memory = reinterpret_cast<ExportFrameFdFunction>(
          dlsym(handle, "rune_visualizer_export_frame_memory"));
      hub->export_frame_semaphore = reinterpret_cast<ExportFrameFdFunction>(
          dlsym(handle, "rune_visualizer_export_frame_semaphore"));
      hub->fall_back_to_readback =
          reinterpret_cast<FallBackToReadbackFunction>(
              dlsym(handle, "rune_visualizer_fall_back_to_readback"));
    }
    g_message(
        "visualizer: libhub.so=%d symbols: frame_info=%d acquire_frame=%d "
        "release_frame=%d set_graphics_device=%d acquire_shared_frame=%d "
        "export_frame_memory=%d export_frame_semaphore=%d "
        "fall_back_to_readback=%d",
        handle != nullptr, hub->frame_info != nullptr,
        hub->acquire_frame != nullptr, hub->release_frame != nullptr,
        hub->set_graphics_device != nullptr,
        hub->acquire_shared_frame != nullptr,
        hub->export_frame_memory != nullptr,
        hub->export_frame_semaphore != nullptr,
        hub->fall_back_to_readback != nullptr);
    return hub;
  }();
  return api;
}

bool hub_api_complete(const HubApi* api) {
  return api->frame_info && api->acquire_frame && api->release_frame;
}

bool hub_can_share(const HubApi* api) {
  return api->set_graphics_device && api->acquire_shared_frame &&
         api->export_frame_memory && api->export_frame_semaphore &&
         api->fall_back_to_readback;
}

// Set once OpenGL failed to import a shared frame: later textures read frames
// back from the start.
std::atomic<bool> sharing_failed{false};

// What Flutter's OpenGL context can do with Vulkan frames.
struct GlProbe {
  // Whether it imports Vulkan memory and semaphores from file descriptors.
  bool usable = false;
  uint8_t device_uuid[GL_UUID_SIZE_EXT] = {};
  uint8_t driver_uuid[GL_UUID_SIZE_EXT] = {};
};

// The EGL display Flutter's engine renders on (see the engine's
// fl_opengl_manager.cc): EGL hands out one display per platform and native
// display.
EGLDisplay flutter_egl_display() {
  GdkDisplay* display = gdk_display_get_default();
#ifdef GDK_WINDOWING_WAYLAND
  if (GDK_IS_WAYLAND_DISPLAY(display)) {
    return eglGetPlatformDisplayEXT(EGL_PLATFORM_WAYLAND_EXT,
                                    gdk_wayland_display_get_wl_display(display),
                                    nullptr);
  }
#endif
#ifdef GDK_WINDOWING_X11
  if (GDK_IS_X11_DISPLAY(display)) {
    return eglGetPlatformDisplayEXT(EGL_PLATFORM_X11_EXT,
                                    gdk_x11_display_get_xdisplay(display),
                                    nullptr);
  }
#endif
  return EGL_NO_DISPLAY;
}

// Whether the context current on this thread imports Vulkan memory and
// semaphores from file descriptors. Their functions must not be called
// otherwise: epoxy aborts on a function the context does not have.
bool has_import_extensions() {
  for (const char* extension :
       {"GL_EXT_memory_object", "GL_EXT_memory_object_fd", "GL_EXT_semaphore",
        "GL_EXT_semaphore_fd"}) {
    if (!epoxy_has_gl_extension(extension)) {
      g_message("visualizer: OpenGL lacks %s; frames are read back",
                extension);
      return false;
    }
  }
  return true;
}

// Reads what the context current on this thread supports.
void probe_current_context(GlProbe* probe) {
  g_message("visualizer: OpenGL renderer %s, version %s",
            reinterpret_cast<const char*>(glGetString(GL_RENDERER)),
            reinterpret_cast<const char*>(glGetString(GL_VERSION)));
  if (!has_import_extensions()) {
    return;
  }
  // A context on several GPUs (a device group) is not supported.
  GLint devices = 0;
  glGetIntegerv(GL_NUM_DEVICE_UUIDS_EXT, &devices);
  if (devices != 1) {
    g_message("visualizer: OpenGL runs on %d GPUs; frames are read back",
              devices);
    return;
  }
  glGetUnsignedBytei_vEXT(GL_DEVICE_UUID_EXT, 0, probe->device_uuid);
  glGetUnsignedBytevEXT(GL_DRIVER_UUID_EXT, probe->driver_uuid);
  probe->usable = glGetError() == GL_NO_ERROR;
}

// Probes a throwaway context on Flutter's EGL display, made like the engine's
// own (OpenGL ES 2 requested), so that it has the same GPU, driver and
// extensions. Restores whatever context this thread had current.
GlProbe probe_gl() {
  GlProbe probe;
  if (g_strcmp0(g_getenv("FLUTTER_LINUX_RENDERER"), "software") == 0) {
    g_message("visualizer: Flutter renders in software; frames are read back");
    return probe;
  }
  EGLDisplay display = flutter_egl_display();
  // Initialising a display again does nothing. It is never terminated here:
  // the engine uses it.
  if (display == EGL_NO_DISPLAY ||
      eglInitialize(display, nullptr, nullptr) != EGL_TRUE) {
    g_message("visualizer: no EGL display; frames are read back");
    return probe;
  }
  // The current context is tracked per client API, so the API is restored
  // first.
  EGLenum api = eglQueryAPI();
  EGLDisplay previous_display = eglGetCurrentDisplay();
  EGLContext previous_context = eglGetCurrentContext();
  EGLSurface previous_draw = eglGetCurrentSurface(EGL_DRAW);
  EGLSurface previous_read = eglGetCurrentSurface(EGL_READ);

  eglBindAPI(EGL_OPENGL_ES_API);
  const EGLint config_attributes[] = {EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
                                      EGL_NONE};
  const EGLint context_attributes[] = {EGL_CONTEXT_CLIENT_VERSION, 2,
                                       EGL_NONE};
  EGLConfig config = nullptr;
  EGLint configs = 0;
  EGLContext context = EGL_NO_CONTEXT;
  if (eglChooseConfig(display, config_attributes, &config, 1, &configs) ==
          EGL_TRUE &&
      configs > 0) {
    context = eglCreateContext(display, config, EGL_NO_CONTEXT,
                               context_attributes);
  }
  // Without a surface, as the engine does (EGL_KHR_surfaceless_context).
  if (context != EGL_NO_CONTEXT &&
      eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, context) ==
          EGL_TRUE) {
    probe_current_context(&probe);
    eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
  } else {
    g_message("visualizer: cannot probe OpenGL (EGL error 0x%x); frames are "
              "read back",
              eglGetError());
  }
  if (context != EGL_NO_CONTEXT) {
    eglDestroyContext(display, context);
  }

  eglBindAPI(api);
  if (previous_context != EGL_NO_CONTEXT) {
    eglMakeCurrent(previous_display, previous_draw, previous_read,
                   previous_context);
  }
  return probe;
}

// The probe of Flutter's OpenGL context, made on first use: the GPU and driver
// do not change while the app runs.
const GlProbe& get_gl_probe() {
  static const GlProbe probe = probe_gl();
  return probe;
}

// Discards OpenGL errors raised before, so that the next check sees only its
// own. Bounded: a lost context can report errors forever.
void clear_gl_errors() {
  for (int i = 0; i < 16 && glGetError() != GL_NO_ERROR; ++i) {
  }
}

// OpenGL objects that only a populate can delete, since Flutter's context is
// current only there: those of GPU textures that were disposed.
struct GlGarbage {
  std::vector<GLuint> textures;
  std::vector<GLuint> memory_objects;
  std::vector<GLuint> semaphores;
};
std::mutex garbage_mutex;
GlGarbage garbage;  // guarded by |garbage_mutex|

// Deletes the garbage. Flutter's context must be current.
void delete_gl_garbage() {
  GlGarbage dead;
  {
    std::lock_guard<std::mutex> lock(garbage_mutex);
    std::swap(dead, garbage);
  }
  // Textures before the memory they are stored in.
  if (!dead.textures.empty()) {
    glDeleteTextures(static_cast<GLsizei>(dead.textures.size()),
                     dead.textures.data());
  }
  if (!dead.memory_objects.empty()) {
    glDeleteMemoryObjectsEXT(static_cast<GLsizei>(dead.memory_objects.size()),
                             dead.memory_objects.data());
  }
  if (!dead.semaphores.empty()) {
    glDeleteSemaphoresEXT(static_cast<GLsizei>(dead.semaphores.size()),
                          dead.semaphores.data());
  }
}

// The memory of a shared image imported into OpenGL, and the texture on it.
struct ImportedImage {
  uint64_t image_id;
  uint64_t image_set;
  GLuint memory;
  GLuint texture;
};

// Imports the memory of |frame|'s image and makes a texture on it, or returns
// false. Flutter's context must be current.
bool import_image(const HubApi* api,
                  const void* handle,
                  const RuneVisualizerSharedFrame& frame,
                  ImportedImage* image) {
  int fd = api->export_frame_memory(handle);
  if (fd < 0) {
    return false;
  }
  clear_gl_errors();
  glCreateMemoryObjectsEXT(1, &image->memory);
  // Vulkan allocated the memory for the image alone.
  const GLint dedicated = GL_TRUE;
  glMemoryObjectParameterivEXT(image->memory, GL_DEDICATED_MEMORY_OBJECT_EXT,
                               &dedicated);
  glImportMemoryFdEXT(image->memory, frame.memory_size,
                      GL_HANDLE_TYPE_OPAQUE_FD_EXT, fd);
  if (glGetError() != GL_NO_ERROR) {
    // Only a successful import takes the descriptor over.
    close(fd);
    glDeleteMemoryObjectsEXT(1, &image->memory);
    return false;
  }

  GLint bound = 0;
  glGetIntegerv(GL_TEXTURE_BINDING_2D, &bound);
  glGenTextures(1, &image->texture);
  glBindTexture(GL_TEXTURE_2D, image->texture);
  // Matches the image's VK_IMAGE_TILING_OPTIMAL (also the default).
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_TILING_EXT, GL_OPTIMAL_TILING_EXT);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
  glTexStorageMem2DEXT(GL_TEXTURE_2D, 1, GL_RGBA8, frame.width, frame.height,
                       image->memory, 0);
  bool stored = glGetError() == GL_NO_ERROR;
  glBindTexture(GL_TEXTURE_2D, bound);
  if (!stored) {
    glDeleteTextures(1, &image->texture);
    glDeleteMemoryObjectsEXT(1, &image->memory);
    return false;
  }
  return true;
}

// The state of a GPU texture, shared by the raster thread (populate) and the
// main thread (retire).
struct GpuTextureState {
  std::mutex mutex;
  // Everything below is guarded by |mutex|.
  // Unregistered: nothing is handed out any more.
  bool retired = false;
  // Whether Flutter's own context was checked for the import extensions, which
  // the probe found on a context like it, and whether it has them.
  bool checked = false;
  bool importable = false;
  // OpenGL failed to import a frame, or cannot: frames are no longer imported.
  bool failed = false;
  std::vector<ImportedImage> images;
  // The hub frame handed to the engine last, held until a newer one replaces
  // it or the texture is retired, and what describes it.
  const void* frame = nullptr;
  uint64_t frame_id = 0;
  uint64_t frame_image_id = 0;
  uint32_t width = 0;
  uint32_t height = 0;
  GLuint texture = 0;
  // The semaphore waited for before the frame was handed over.
  GLuint semaphore = 0;
  // The image of the frame handed over before, which the engine may have
  // sampled until this populate.
  uint64_t previous_image_id = 0;
  guint64 handed_over = 0;
};

// Makes |frame| the frame handed to the engine: imports its image unless it
// was imported before, and waits for the semaphore of the frame, which makes
// Vulkan's writes visible and tells OpenGL the layout Vulkan left the image in
// (VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL). Gives the previous frame back
// and deletes the textures of images that are no longer drawn into. Returns
// false, with nothing changed and |handle| still the caller's, if OpenGL cannot
// take the frame. Flutter's context must be current.
bool hand_over(GpuTextureState* state,
               const HubApi* api,
               const void* handle,
               const RuneVisualizerSharedFrame& frame) {
  ImportedImage* image = nullptr;
  for (ImportedImage& imported : state->images) {
    if (imported.image_id == frame.image_id) {
      image = &imported;
    }
  }
  if (image == nullptr) {
    ImportedImage imported = {frame.image_id, frame.image_set, 0, 0};
    if (!import_image(api, handle, frame, &imported)) {
      g_warning("visualizer: OpenGL cannot import image %" G_GUINT64_FORMAT,
                static_cast<guint64>(frame.image_id));
      return false;
    }
    state->images.push_back(imported);
    image = &state->images.back();
  }

  int fd = api->export_frame_semaphore(handle);
  if (fd < 0) {
    return false;
  }
  clear_gl_errors();
  GLuint semaphore = 0;
  glGenSemaphoresEXT(1, &semaphore);
  glImportSemaphoreFdEXT(semaphore, GL_HANDLE_TYPE_OPAQUE_FD_EXT, fd);
  if (glGetError() != GL_NO_ERROR) {
    // Only a successful import takes the descriptor over.
    close(fd);
    glDeleteSemaphoresEXT(1, &semaphore);
    g_warning("visualizer: OpenGL cannot import a semaphore");
    return false;
  }
  const GLenum layout = GL_LAYOUT_SHADER_READ_ONLY_EXT;
  glWaitSemaphoreEXT(semaphore, 0, nullptr, 1, &image->texture, &layout);
  if (glGetError() != GL_NO_ERROR) {
    glDeleteSemaphoresEXT(1, &semaphore);
    g_warning("visualizer: OpenGL cannot wait for a semaphore");
    return false;
  }

  // The wait for the previous frame was issued long ago.
  if (state->semaphore != 0) {
    glDeleteSemaphoresEXT(1, &state->semaphore);
  }
  if (state->frame != nullptr) {
    api->release_frame(state->frame);
  }
  state->previous_image_id = state->frame_image_id;
  state->frame = handle;
  state->frame_id = frame.id;
  state->frame_image_id = frame.image_id;
  state->width = frame.width;
  state->height = frame.height;
  state->texture = image->texture;
  state->semaphore = semaphore;

  // Images of an earlier set are gone from Rust (apart from frames still
  // held); their textures go once the engine stopped sampling them, which it
  // may still do with the frame handed over before this one.
  std::vector<ImportedImage> kept;
  for (const ImportedImage& imported : state->images) {
    if (imported.image_set == frame.image_set ||
        imported.image_id == state->frame_image_id ||
        imported.image_id == state->previous_image_id) {
      kept.push_back(imported);
    } else {
      glDeleteTextures(1, &imported.texture);
      glDeleteMemoryObjectsEXT(1, &imported.memory);
    }
  }
  state->images = std::move(kept);

  if (should_log(++state->handed_over)) {
    g_message("visualizer: populate #%" G_GUINT64_FORMAT
              ": frame %" G_GUINT64_FORMAT " shared (image %" G_GUINT64_FORMAT
              ", %ux%u)",
              state->handed_over, static_cast<guint64>(frame.id),
              static_cast<guint64>(frame.image_id), frame.width, frame.height);
  }
  return true;
}

}  // namespace

// A Flutter texture fed by the Rust visualizer with read-back frames. The
// engine calls copy_pixels on its raster thread, with the GL context current,
// and uploads the returned pixels with glTexImage2D(GL_RGBA) straight
// afterwards. The texture holds a hub frame between two calls, so only the
// raster thread touches its fields; dispose runs after the last reference,
// when no call can be in flight.
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

// A Flutter texture fed by the Rust visualizer with frames in Vulkan images,
// whose memory it imports into Flutter's OpenGL context without a copy. The
// engine calls populate on its raster thread with that context current, and
// keeps sampling the texture it got until populate hands out another one, so
// the frame is held until then. OpenGL objects can only be deleted in
// populate: a retired texture leaves them to the next populate of any GPU
// texture.
G_DECLARE_FINAL_TYPE(VisualizerGpuTexture,
                     visualizer_gpu_texture,
                     VISUALIZER,
                     GPU_TEXTURE,
                     FlTextureGL)

struct _VisualizerGpuTexture {
  FlTextureGL parent_instance;

  GpuTextureState* state;  // owned
};

G_DEFINE_TYPE(VisualizerGpuTexture,
              visualizer_gpu_texture,
              fl_texture_gl_get_type())

// Implements FlTextureGL::populate. Hands out the newest shared frame, or the
// one handed out last if there is no newer one (for example after the
// visualizer fell back to reading frames back, until the app shows the pixel
// texture instead).
static gboolean visualizer_gpu_texture_populate(FlTextureGL* texture,
                                                uint32_t* target,
                                                uint32_t* name,
                                                uint32_t* width,
                                                uint32_t* height,
                                                GError** error) {
  GpuTextureState* state = VISUALIZER_GPU_TEXTURE(texture)->state;
  std::lock_guard<std::mutex> lock(state->mutex);
  if (state->retired) {
    g_set_error(error, g_quark_from_static_string("visualizer"), 0,
                "texture disposed");
    return FALSE;
  }
  const HubApi* api = get_hub_api();
  if (!state->checked) {
    state->checked = true;
    state->importable = has_import_extensions();
    if (!state->importable) {
      state->failed = true;
      sharing_failed = true;
      api->fall_back_to_readback();
    }
  }
  if (state->importable) {
    delete_gl_garbage();
  }

  RuneVisualizerSharedFrame frame = {};
  const void* handle =
      state->failed ? nullptr : api->acquire_shared_frame(&frame);
  if (handle != nullptr && state->frame != nullptr &&
      frame.id == state->frame_id) {
    // Already handed over.
    api->release_frame(handle);
  } else if (handle != nullptr && !hand_over(state, api, handle, frame)) {
    // Shows the last frame until the app changes to the pixel texture.
    api->release_frame(handle);
    state->failed = true;
    sharing_failed = true;
    g_warning("visualizer: OpenGL cannot take shared frames; reading frames "
              "back");
    api->fall_back_to_readback();
  }

  if (state->frame == nullptr) {
    g_set_error(error, g_quark_from_static_string("visualizer"), 0,
                "no frame yet");
    return FALSE;
  }
  *target = GL_TEXTURE_2D;
  *name = state->texture;
  *width = state->width;
  *height = state->height;
  return TRUE;
}

// Stops handing out frames, gives the held frame back and leaves the OpenGL
// objects to the next populate. Called once the texture is unregistered.
static void visualizer_gpu_texture_retire(VisualizerGpuTexture* self) {
  GpuTextureState* state = self->state;
  std::lock_guard<std::mutex> lock(state->mutex);
  if (state->retired) {
    return;
  }
  state->retired = true;
  {
    std::lock_guard<std::mutex> garbage_lock(garbage_mutex);
    for (const ImportedImage& imported : state->images) {
      garbage.textures.push_back(imported.texture);
      garbage.memory_objects.push_back(imported.memory);
    }
    if (state->semaphore != 0) {
      garbage.semaphores.push_back(state->semaphore);
    }
  }
  state->images.clear();
  state->semaphore = 0;
  if (state->frame != nullptr) {
    get_hub_api()->release_frame(state->frame);
    state->frame = nullptr;
  }
  g_message("visualizer: GPU texture retired after %" G_GUINT64_FORMAT
            " frames handed over",
            state->handed_over);
}

// Implements GObject::dispose.
static void visualizer_gpu_texture_dispose(GObject* object) {
  visualizer_gpu_texture_retire(VISUALIZER_GPU_TEXTURE(object));
  G_OBJECT_CLASS(visualizer_gpu_texture_parent_class)->dispose(object);
}

// Implements GObject::finalize.
static void visualizer_gpu_texture_finalize(GObject* object) {
  VisualizerGpuTexture* self = VISUALIZER_GPU_TEXTURE(object);
  delete self->state;
  self->state = nullptr;
  G_OBJECT_CLASS(visualizer_gpu_texture_parent_class)->finalize(object);
}

static void visualizer_gpu_texture_class_init(
    VisualizerGpuTextureClass* klass) {
  G_OBJECT_CLASS(klass)->dispose = visualizer_gpu_texture_dispose;
  G_OBJECT_CLASS(klass)->finalize = visualizer_gpu_texture_finalize;
  FL_TEXTURE_GL_CLASS(klass)->populate = visualizer_gpu_texture_populate;
}

static void visualizer_gpu_texture_init(VisualizerGpuTexture* self) {
  self->state = new GpuTextureState();
}

struct _VisualizerChannel {
  FlTextureRegistrar* registrar;
  FlMethodChannel* channel;
  VisualizerTexture* texture;  // owned, null if there is none
  // Owned; null if there is none or frames cannot be shared.
  VisualizerGpuTexture* gpu_texture;
  // The GPU texture unregistered last, owned: kept alive until the next
  // dispose, since the engine looks textures up without a reference and the
  // raster thread may still be inside its populate.
  VisualizerGpuTexture* retired_gpu_texture;
  guint refresh_source;  // GSource id of the poll, 0 if none
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
    if (self->gpu_texture != nullptr) {
      fl_texture_registrar_mark_texture_frame_available(
          self->registrar, FL_TEXTURE(self->gpu_texture));
    }
  }
  return G_SOURCE_CONTINUE;
}

// Stops refreshing and unregisters the textures, if there are any.
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
  if (self->gpu_texture != nullptr) {
    fl_texture_registrar_unregister_texture(self->registrar,
                                            FL_TEXTURE(self->gpu_texture));
    visualizer_gpu_texture_retire(self->gpu_texture);
    g_clear_object(&self->retired_gpu_texture);
    self->retired_gpu_texture = self->gpu_texture;
    self->gpu_texture = nullptr;
  }
}

// Registers the GPU texture if frames can be shared, after telling the hub
// which GPU Flutter's OpenGL context runs on: the app starts the visualizer
// once `create` returns, and it chooses its GPU then.
static void visualizer_channel_create_gpu_texture(VisualizerChannel* self) {
  const HubApi* api = get_hub_api();
  if (!hub_can_share(api)) {
    return;
  }
  const GlProbe& probe = get_gl_probe();
  bool share = probe.usable && !sharing_failed;
  // Unknown clears what an earlier `create` set.
  api->set_graphics_device(share ? probe.device_uuid : nullptr,
                           share ? probe.driver_uuid : nullptr);
  if (!share) {
    return;
  }
  self->gpu_texture = VISUALIZER_GPU_TEXTURE(
      g_object_new(visualizer_gpu_texture_get_type(), nullptr));
  if (!fl_texture_registrar_register_texture(self->registrar,
                                             FL_TEXTURE(self->gpu_texture))) {
    g_clear_object(&self->gpu_texture);
    g_warning("visualizer: the engine refused the GPU texture");
    api->set_graphics_device(nullptr, nullptr);
  }
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
  visualizer_channel_create_gpu_texture(self);
  int64_t id = fl_texture_get_id(FL_TEXTURE(self->texture));
  int64_t gpu_id = self->gpu_texture != nullptr
                       ? fl_texture_get_id(FL_TEXTURE(self->gpu_texture))
                       : -1;
  g_message("visualizer: registered textures: pixels %" G_GINT64_FORMAT
            ", gpu %" G_GINT64_FORMAT,
            id, gpu_id);

  self->announced_frame_id = 0;
  self->announcements = 0;
  // Linux has no display link for the embedder; poll a little faster than 120 Hz.
  self->refresh_source = g_timeout_add(8, visualizer_channel_refresh, self);

  g_autoptr(FlValue) result = fl_value_new_map();
  fl_value_set_string_take(result, "pixels", fl_value_new_int(id));
  if (self->gpu_texture != nullptr) {
    fl_value_set_string_take(result, "gpu", fl_value_new_int(gpu_id));
  }
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
  g_clear_object(&self->retired_gpu_texture);
  g_clear_object(&self->channel);
  g_clear_object(&self->registrar);
  g_free(self);
}
