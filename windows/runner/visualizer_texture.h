#ifndef RUNNER_VISUALIZER_TEXTURE_H_
#define RUNNER_VISUALIZER_TEXTURE_H_

#include <flutter/binary_messenger.h>
#include <flutter/encodable_value.h>
#include <flutter/method_channel.h>
#include <flutter/texture_registrar.h>

#include <memory>

class VisualizerTexture;

// Owns the single visualizer texture and serves the `not.ci.rune/visualizer`
// method channel (`create` returns the texture id, `dispose` releases it).
// Must be created, used and destroyed on the platform thread.
class VisualizerChannel {
 public:
  // |messenger| and |registrar| must outlive this object.
  VisualizerChannel(flutter::BinaryMessenger* messenger,
                    flutter::TextureRegistrar* registrar);
  ~VisualizerChannel();

  VisualizerChannel(const VisualizerChannel&) = delete;
  VisualizerChannel& operator=(const VisualizerChannel&) = delete;

 private:
  flutter::TextureRegistrar* registrar_;
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> channel_;
  std::unique_ptr<VisualizerTexture> texture_;
};

#endif  // RUNNER_VISUALIZER_TEXTURE_H_
