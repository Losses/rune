#include "flutter_window.h"

#include <flutter/standard_method_codec.h>

#include <optional>

#include "flutter/generated_plugin_registrant.h"

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());

  // Scene files: paths queue until Dart has called takePending once, then they
  // are sent with "open".
  scene_channel_ =
      std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
          flutter_controller_->engine()->messenger(), "ci.not.rune/scene_files",
          &flutter::StandardMethodCodec::GetInstance());
  scene_channel_->SetMethodCallHandler(
      [this](const flutter::MethodCall<flutter::EncodableValue>& call,
             std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>>
                 result) {
        if (call.method_name() == "takePending") {
          flutter::EncodableList paths;
          for (const std::string& path : pending_scene_files_) {
            paths.push_back(flutter::EncodableValue(path));
          }
          pending_scene_files_.clear();
          dart_listening_ = true;
          result->Success(flutter::EncodableValue(paths));
        } else {
          result->NotImplemented();
        }
      });
  // The Rust hub registers the visualizer's texture with the engine itself. Its
  // symbols are looked up at run time: the Dart side (rinf) loads hub.dll as
  // well, and a missing library or symbol only leaves the visualizer without a
  // texture.
  HMODULE hub = GetModuleHandleW(L"hub.dll");
  if (!hub) {
    hub = LoadLibraryW(L"hub.dll");
  }
  if (hub) {
    if (auto attach = reinterpret_cast<bool (*)(void*)>(
            GetProcAddress(hub, "rune_flutter_attach"))) {
      attach(flutter_controller_->engine()->GetRegistrarForPlugin(
          "RuneFlutterTexture"));
    }
  }
  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    this->Show();
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

void FlutterWindow::ReceiveSceneFile(const std::string& path) {
  if (dart_listening_ && scene_channel_) {
    flutter::EncodableList paths;
    paths.push_back(flutter::EncodableValue(path));
    scene_channel_->InvokeMethod(
        "open", std::make_unique<flutter::EncodableValue>(paths));
  } else {
    pending_scene_files_.push_back(path);
  }
}

void FlutterWindow::OnDestroy() {
  scene_channel_ = nullptr;
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_COPYDATA: {
      // A second instance forwards the scene file it was asked to open.
      const auto* data = reinterpret_cast<const COPYDATASTRUCT*>(lparam);
      if (data && data->dwData == 0x52554E45 && data->lpData) {
        ReceiveSceneFile(std::string(static_cast<const char*>(data->lpData),
                                     data->cbData));
        return TRUE;
      }
      break;
    }
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
