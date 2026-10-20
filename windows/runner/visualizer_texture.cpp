#include "visualizer_texture.h"

#include <windows.h>
// Must come after windows.h.
#include <timeapi.h>

#include <flutter/standard_method_codec.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstdarg>
#include <cstring>
#include <mutex>
#include <string>
#include <thread>
#include <utility>
#include <vector>

namespace {

// Diagnostics for the texture bridge; they show up in a debugger or DebugView.
void Log(const char* format, ...) {
  char message[512];
  va_list args;
  va_start(args, format);
  vsnprintf(message, sizeof(message), format, args);
  va_end(args);
  std::string line = std::string("[rune.visualizer] ") + message + "\n";
  OutputDebugStringA(line.c_str());
}

// Log the first few occurrences of an event, then every 300th.
bool ShouldLog(uint64_t count) { return count <= 3 || count % 300 == 0; }

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

// Resolves the hub's symbols once. The Dart side (rinf) loads hub.dll itself,
// so it is normally in the process already; resolving at run time avoids a
// link-time dependency and tolerates missing symbols.
const HubApi* GetHubApi() {
  static const HubApi* api = [] {
    HMODULE module = GetModuleHandleW(L"hub.dll");
    if (!module) {
      module = LoadLibraryW(L"hub.dll");
    }
    auto* hub = new HubApi();
    if (module) {
      hub->frame_info = reinterpret_cast<FrameInfoFunction>(
          GetProcAddress(module, "rune_visualizer_frame_info"));
      hub->acquire_frame = reinterpret_cast<AcquireFrameFunction>(
          GetProcAddress(module, "rune_visualizer_acquire_frame"));
      hub->release_frame = reinterpret_cast<ReleaseFrameFunction>(
          GetProcAddress(module, "rune_visualizer_release_frame"));
    }
    Log("hub.dll=%d symbols: frame_info=%d acquire_frame=%d release_frame=%d",
        module != nullptr, hub->frame_info != nullptr,
        hub->acquire_frame != nullptr, hub->release_frame != nullptr);
    return hub;
  }();
  return api;
}

bool HubApiComplete(const HubApi* api) {
  return api->frame_info && api->acquire_frame && api->release_frame;
}

// A pixel buffer handed to Flutter together with whatever keeps it alive. The
// engine calls |Release| right after uploading the pixels to the GPU.
struct HeldPixelBuffer {
  FlutterDesktopPixelBuffer buffer = {};
  // The hub frame whose pixels |buffer| points into, or null if they are in
  // |copy|.
  const void* frame = nullptr;
  std::vector<uint8_t> copy;

  static void Release(void* context) {
    auto* held = static_cast<HeldPixelBuffer*>(context);
    if (held->frame) {
      GetHubApi()->release_frame(held->frame);
    }
    delete held;
  }
};

}  // namespace

// A Flutter texture fed by the Rust visualizer. The pixel callback runs on the
// raster thread and only touches immutable state and |disposed_|; the refresh
// thread only calls the thread-safe MarkTextureFrameAvailable.
class VisualizerTexture {
 public:
  // Registers a texture, or returns null and sets |error| if the hub is
  // unusable.
  static std::unique_ptr<VisualizerTexture> Create(
      flutter::TextureRegistrar* registrar, std::string* error) {
    const HubApi* api = GetHubApi();
    if (!HubApiComplete(api)) {
      *error = "The visualizer symbols are missing from hub.dll.";
      return nullptr;
    }
    return std::unique_ptr<VisualizerTexture>(
        new VisualizerTexture(registrar, api));
  }

  ~VisualizerTexture() {
    // Stop announcing first: MarkTextureFrameAvailable must not run on an
    // unregistered texture.
    {
      std::lock_guard<std::mutex> lock(mutex_);
      stopping_ = true;
    }
    wake_.notify_all();
    if (refresh_thread_.joinable()) {
      refresh_thread_.join();
    }
    Log("dispose texture %lld after %llu frames announced",
        static_cast<long long>(texture_id_),
        static_cast<unsigned long long>(announcements_));
    // From here on the pixel callback hands out nothing.
    state_->disposed = true;
    // Unregistration is asynchronous: the raster thread may still be inside
    // the callback, which reads the texture object, so keep it alive until the
    // engine confirms.
    std::shared_ptr<flutter::TextureVariant> texture = std::move(texture_);
    registrar_->UnregisterTexture(texture_id_, [texture]() {});
  }

  int64_t id() const { return texture_id_; }

 private:
  // State shared with the raster thread's callback.
  struct SharedState {
    const HubApi* api;
    std::atomic<bool> disposed{false};
    std::atomic<uint64_t> handed_over{0};
    std::atomic<uint64_t> copies{0};
  };

  VisualizerTexture(flutter::TextureRegistrar* registrar, const HubApi* api)
      : registrar_(registrar), state_(std::make_shared<SharedState>()) {
    state_->api = api;
    std::shared_ptr<SharedState> state = state_;
    texture_ = std::make_shared<flutter::TextureVariant>(
        flutter::PixelBufferTexture([state](size_t, size_t) {
          return CopyPixelBuffer(*state);
        }));
    texture_id_ = registrar_->RegisterTexture(texture_.get());
    Log("registered texture %lld", static_cast<long long>(texture_id_));
    refresh_thread_ = std::thread([this] { Refresh(); });
  }

  // Called on the raster thread. Hands out the newest frame without copying
  // when its rows are tight; the engine releases it after the GPU upload.
  static const FlutterDesktopPixelBuffer* CopyPixelBuffer(SharedState& state) {
    if (state.disposed) {
      return nullptr;
    }
    RuneVisualizerFrame frame = {};
    const void* handle = state.api->acquire_frame(&frame);
    if (!handle) {
      return nullptr;
    }
    auto* held = new HeldPixelBuffer();
    held->buffer.width = frame.width;
    held->buffer.height = frame.height;
    held->buffer.release_callback = HeldPixelBuffer::Release;
    held->buffer.release_context = held;
    const size_t row = static_cast<size_t>(frame.width) * 4;
    if (frame.stride == row) {
      held->frame = handle;
      held->buffer.buffer = frame.pixels;
      uint64_t count = ++state.handed_over;
      if (ShouldLog(count)) {
        Log("copyPixelBuffer #%llu: frame %llu handed over without a copy "
            "(%ux%u)",
            static_cast<unsigned long long>(count),
            static_cast<unsigned long long>(frame.id), frame.width,
            frame.height);
      }
    } else {
      // Padded rows: the engine reads tightly packed RGBA, so copy and give
      // the renderer its buffer back at once.
      held->copy.resize(row * frame.height);
      for (uint32_t y = 0; y < frame.height; ++y) {
        std::memcpy(held->copy.data() + y * row, frame.pixels + y * frame.stride,
                    row);
      }
      state.api->release_frame(handle);
      held->buffer.buffer = held->copy.data();
      uint64_t count = ++state.copies;
      if (ShouldLog(count)) {
        Log("copyPixelBuffer #%llu: frame %llu copied (%ux%u, stride %zu)",
            static_cast<unsigned long long>(count),
            static_cast<unsigned long long>(frame.id), frame.width,
            frame.height, frame.stride);
      }
    }
    return &held->buffer;
  }

  // Polls the hub for new frames. Windows has no display link, so the thread
  // wakes every few milliseconds; timeBeginPeriod(1) lifts the default ~15.6 ms
  // timer granularity for the thread's lifetime.
  void Refresh() {
    timeBeginPeriod(1);
    uint64_t announced = 0;
    std::unique_lock<std::mutex> lock(mutex_);
    while (!stopping_) {
      uint64_t frame_id = state_->api->frame_info(nullptr, nullptr);
      if (frame_id != 0 && frame_id != announced) {
        announced = frame_id;
        ++announcements_;
        if (ShouldLog(announcements_)) {
          Log("frame available #%llu: frame %llu texture %lld",
              static_cast<unsigned long long>(announcements_),
              static_cast<unsigned long long>(frame_id),
              static_cast<long long>(texture_id_));
        }
        registrar_->MarkTextureFrameAvailable(texture_id_);
      }
      wake_.wait_for(lock, std::chrono::milliseconds(6),
                     [this] { return stopping_; });
    }
    timeEndPeriod(1);
  }

  flutter::TextureRegistrar* registrar_;
  std::shared_ptr<SharedState> state_;
  // Shared so that the unregistration callback can keep it alive.
  std::shared_ptr<flutter::TextureVariant> texture_;
  int64_t texture_id_ = 0;
  std::thread refresh_thread_;
  std::mutex mutex_;
  std::condition_variable wake_;
  bool stopping_ = false;  // guarded by |mutex_|
  uint64_t announcements_ = 0;  // refresh thread only, read after join
};

VisualizerChannel::VisualizerChannel(flutter::BinaryMessenger* messenger,
                                     flutter::TextureRegistrar* registrar)
    : registrar_(registrar),
      channel_(
          std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
              messenger, "not.ci.rune/visualizer",
              &flutter::StandardMethodCodec::GetInstance())) {
  // Handlers run on the platform thread; this object outlives the channel.
  channel_->SetMethodCallHandler(
      [this](const flutter::MethodCall<flutter::EncodableValue>& call,
             std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>>
                 result) {
        if (call.method_name() == "create") {
          texture_.reset();
          std::string error;
          texture_ = VisualizerTexture::Create(registrar_, &error);
          if (!texture_) {
            Log("create failed: %s", error.c_str());
            result->Error("unavailable", error);
            return;
          }
          result->Success(flutter::EncodableValue(texture_->id()));
        } else if (call.method_name() == "dispose") {
          texture_.reset();
          result->Success();
        } else {
          result->NotImplemented();
        }
      });
  Log("channel registered");
}

VisualizerChannel::~VisualizerChannel() {
  channel_->SetMethodCallHandler(nullptr);
  texture_.reset();
}
