#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>
#include <shellapi.h>

#include <string>
#include <vector>

#include "flutter_window.h"
#include "utils.h"

#include <bitsdojo_window_windows/bitsdojo_window_plugin.h>
auto bdw = bitsdojo_window_configure(BDW_CUSTOM_FRAME | BDW_HIDE_ON_STARTUP);

namespace {

bool IsSceneFile(const std::string& path) {
  const std::string extension = ".runescene";
  if (path.size() < extension.size()) {
    return false;
  }
  for (size_t i = 0; i < extension.size(); i++) {
    char c = path[path.size() - extension.size() + i];
    if (c >= 'A' && c <= 'Z') {
      c = static_cast<char>(c - 'A' + 'a');
    }
    if (c != extension[i]) {
      return false;
    }
  }
  return true;
}

// The path as an absolute UTF-8 path. The running instance has its own working
// directory, so a path relative to this process would resolve to another file.
std::string AbsolutePathUtf8(const wchar_t* path) {
  DWORD length = ::GetFullPathNameW(path, 0, nullptr, nullptr);
  if (length == 0) {
    return Utf8FromUtf16(path);
  }
  std::wstring full(length, L'\0');
  length = ::GetFullPathNameW(path, length, &full[0], nullptr);
  if (length == 0 || length >= full.size()) {
    return Utf8FromUtf16(path);
  }
  full.resize(length);
  return Utf8FromUtf16(full.c_str());
}

}  // namespace

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  // Create a mutex
  HANDLE hMutex = CreateMutex(NULL, TRUE, L"Rune_SingleInstance_Mutex");
  DWORD dwError = GetLastError();
  
  // If the mutex already exists, it means another instance is running
  if (dwError == ERROR_ALREADY_EXISTS) {
    // Find the existing window
    HWND hwnd = ::FindWindow(L"FLUTTER_RUNNER_WIN32_WINDOW", L"Rune");
    if (hwnd != NULL) {
      // If the window is minimized, restore it
      if (::IsIconic(hwnd)) {
        ::ShowWindow(hwnd, SW_RESTORE);
      }
      // Bring the window to the foreground
      ::SetForegroundWindow(hwnd);
      // Pass the scene files from the command line to the running instance.
      // SendMessage returns after the receiver handled the message, so the
      // paths are delivered before this process exits.
      int argc = 0;
      wchar_t** argv = ::CommandLineToArgvW(::GetCommandLineW(), &argc);
      for (int i = 1; argv != nullptr && i < argc; i++) {
        if (IsSceneFile(Utf8FromUtf16(argv[i]))) {
          const std::string path = AbsolutePathUtf8(argv[i]);
          COPYDATASTRUCT data;
          data.dwData = kSceneFileCopyData;
          data.cbData = static_cast<DWORD>(path.size());
          data.lpData = const_cast<char*>(path.data());
          ::SendMessage(hwnd, WM_COPYDATA, reinterpret_cast<WPARAM>(hwnd),
                        reinterpret_cast<LPARAM>(&data));
        }
      }
      if (argv != nullptr) {
        ::LocalFree(argv);
      }
    }
    // Release the mutex handle
    ReleaseMutex(hMutex);
    CloseHandle(hMutex);
    return EXIT_FAILURE;
  }

  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"data");

  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1280, 720);
  if (!window.Create(L"Rune", origin, size)) {
    return EXIT_FAILURE;
  }
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();

  ReleaseMutex(hMutex);
  CloseHandle(hMutex);
  return EXIT_SUCCESS;
}
