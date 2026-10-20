#ifndef FLUTTER_VISUALIZER_TEXTURE_H_
#define FLUTTER_VISUALIZER_TEXTURE_H_

#include <flutter_linux/flutter_linux.h>

G_BEGIN_DECLS

/**
 * VisualizerChannel:
 *
 * Owns the single visualizer texture and serves the `not.ci.rune/visualizer`
 * method channel (`create` returns the texture id, `dispose` releases it).
 * Everything runs on the GTK main thread, except the texture's pixel callback,
 * which the engine calls on its raster thread.
 */
typedef struct _VisualizerChannel VisualizerChannel;

/**
 * visualizer_channel_new:
 * @engine: the engine of the view, which must outlive the channel.
 *
 * Registers the method channel.
 *
 * Returns: a new #VisualizerChannel, to be freed with
 * visualizer_channel_free().
 */
VisualizerChannel* visualizer_channel_new(FlEngine* engine);

/**
 * visualizer_channel_free:
 * @self: a #VisualizerChannel.
 *
 * Unregisters the texture and the method channel.
 */
void visualizer_channel_free(VisualizerChannel* self);

G_END_DECLS

#endif  // FLUTTER_VISUALIZER_TEXTURE_H_
