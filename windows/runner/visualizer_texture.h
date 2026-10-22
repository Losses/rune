#ifndef RUNNER_VISUALIZER_TEXTURE_H_
#define RUNNER_VISUALIZER_TEXTURE_H_

#include <flutter/binary_messenger.h>
#include <flutter/encodable_value.h>
#include <flutter/method_channel.h>
#include <flutter/texture_registrar.h>
#include <flutter_plugin_registrar.h>

#include <memory>

class VisualizerTexture;

// Owns the visualizer textures and serves the `not.ci.rune/visualizer` method
// channel: `create` returns `{"pixels": id}` for the texture fed with
// read-back frames, plus `"gpu": id` for the one fed with shared DirectX
// textures when the hub can share them; `dispose` releases both.
// Must be created, used and destroyed on the platform thread.
class VisualizerChannel {
 public:
  // |messenger|, |registrar| and |plugin_registrar| (which tells the adapter
  // Flutter renders on) must outlive this object.
  VisualizerChannel(flutter::BinaryMessenger* messenger,
                    flutter::TextureRegistrar* registrar,
                    FlutterDesktopPluginRegistrarRef plugin_registrar);
  ~VisualizerChannel();

  VisualizerChannel(const VisualizerChannel&) = delete;
  VisualizerChannel& operator=(const VisualizerChannel&) = delete;

 private:
  flutter::TextureRegistrar* registrar_;
  FlutterDesktopPluginRegistrarRef plugin_registrar_;
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> channel_;
  std::unique_ptr<VisualizerTexture> texture_;
};

#endif  // RUNNER_VISUALIZER_TEXTURE_H_
