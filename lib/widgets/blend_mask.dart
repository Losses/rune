import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

// From: https://stackoverflow.com/questions/62620579/how-to-use-mix-blend-mode-in-flutter?noredirect=1&lq=1

// Applies a BlendMode to its child.
class BlendMask extends SingleChildRenderObjectWidget {
  final BlendMode blendMode;
  final double opacity;

  const BlendMask({
    required this.blendMode,
    this.opacity = 1.0,
    super.key,
    super.child,
  });

  @override
  RenderObject createRenderObject(context) {
    return RenderBlendMask(blendMode, opacity);
  }

  @override
  void updateRenderObject(BuildContext context, RenderBlendMask renderObject) {
    renderObject.blendMode = blendMode;
    renderObject.opacity = opacity;
  }
}

class RenderBlendMask extends RenderProxyBox {
  BlendMode blendMode;
  double opacity;

  RenderBlendMask(this.blendMode, this.opacity);

  @override
  void paint(context, offset) {
    // Create a new layer and specify the blend mode and opacity to composite it with:
    context.canvas.saveLayer(
      offset & size,
      Paint()
        ..blendMode = blendMode
        ..color = Color.fromARGB((opacity * 255).round(), 255, 255, 255),
    );

    super.paint(context, offset);

    // Composite the layer back into the canvas using the blendmode:
    context.canvas.restore();
  }
}
