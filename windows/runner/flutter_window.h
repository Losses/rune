#ifndef RUNNER_FLUTTER_WINDOW_H_
#define RUNNER_FLUTTER_WINDOW_H_

#include <flutter/dart_project.h>
#include <flutter/encodable_value.h>
#include <flutter/flutter_view_controller.h>
#include <flutter/method_channel.h>
#include <windows.h>

#include <memory>
#include <string>
#include <vector>

#include "win32_window.h"

// Identifies a scene file path in a WM_COPYDATA message: a second instance
// sends it to the first one.
constexpr ULONG_PTR kSceneFileCopyData = 0x52554E45;

// A window that does nothing but host a Flutter view.
class FlutterWindow : public Win32Window {
 public:
  // Creates a new FlutterWindow hosting a Flutter view running |project|.
  explicit FlutterWindow(const flutter::DartProject& project);
  virtual ~FlutterWindow();

 protected:
  // Win32Window:
  bool OnCreate() override;
  void OnDestroy() override;
  LRESULT MessageHandler(HWND window, UINT const message, WPARAM const wparam,
                         LPARAM const lparam) noexcept override;

 private:
  // The project to run.
  flutter::DartProject project_;

  // The Flutter instance hosted by this window.
  std::unique_ptr<flutter::FlutterViewController> flutter_controller_;

  // Delivers scene files (.runescene) to Dart.
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>>
      scene_channel_;

  // Scene file paths received before Dart called takePending.
  std::vector<std::string> pending_scene_files_;
  bool dart_listening_ = false;

  // Queues the path, or sends it to Dart once Dart is listening.
  void ReceiveSceneFile(const std::string& path);
};

#endif  // RUNNER_FLUTTER_WINDOW_H_
