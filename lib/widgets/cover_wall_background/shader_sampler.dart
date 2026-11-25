// Adapted from AnimatedSampler in package:flutter_shaders
// (Copyright 2013 The Flutter Authors; BSD-style license).

import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:fluent_ui/fluent_ui.dart';

/// Paints [image], a snapshot of the sampler's child, onto [canvas].
typedef ShaderSamplerBuilder =
    void Function(ui.Image image, Size size, Canvas canvas);

/// Shows its child through [builder], which draws a snapshot of the child.
///
/// It is AnimatedSampler from flutter_shaders, except for how a new drawing
/// starts: when [repaint] notifies, this layer alone draws again. An
/// animation that rebuilds the sampler instead rebuilds inside the layout
/// builders above it, and such a rebuild lays out, and so repaints, every box
/// up to the nearest relayout boundary: the whole page, for each frame.
///
/// The snapshot of the child is kept while the child does not paint again, so
/// a drawing that [repaint] starts runs the shader alone. A child with layers
/// of its own can change them without painting here, so its snapshot is taken
/// for every drawing.
class ShaderSampler extends SingleChildRenderObjectWidget {
  const ShaderSampler(
    this.builder, {
    required this.repaint,
    required super.child,
    super.key,
  });

  final ShaderSamplerBuilder builder;

  /// Notifies when [builder] would draw something new.
  final Listenable repaint;

  @override
  RenderShaderSampler createRenderObject(BuildContext context) {
    return RenderShaderSampler(
      devicePixelRatio: MediaQuery.devicePixelRatioOf(context),
      builder: builder,
      repaint: repaint,
    );
  }

  @override
  void updateRenderObject(
    BuildContext context,
    RenderShaderSampler renderObject,
  ) {
    renderObject
      ..devicePixelRatio = MediaQuery.devicePixelRatioOf(context)
      ..builder = builder
      ..repaint = repaint;
  }
}

class RenderShaderSampler extends RenderProxyBox {
  RenderShaderSampler({
    required double devicePixelRatio,
    required ShaderSamplerBuilder builder,
    required Listenable repaint,
  }) : _devicePixelRatio = devicePixelRatio,
       _builder = builder,
       _repaint = repaint;

  double get devicePixelRatio => _devicePixelRatio;
  double _devicePixelRatio;
  set devicePixelRatio(double value) {
    if (value == _devicePixelRatio) return;
    _devicePixelRatio = value;
    markNeedsCompositedLayerUpdate();
  }

  ShaderSamplerBuilder get builder => _builder;
  ShaderSamplerBuilder _builder;
  set builder(ShaderSamplerBuilder value) {
    if (value == _builder) return;
    _builder = value;
    markNeedsCompositedLayerUpdate();
  }

  Listenable get repaint => _repaint;
  Listenable _repaint;
  set repaint(Listenable value) {
    if (value == _repaint) return;
    if (attached) {
      _repaint.removeListener(markNeedsCompositedLayerUpdate);
      value.addListener(markNeedsCompositedLayerUpdate);
    }
    _repaint = value;
    markNeedsCompositedLayerUpdate();
  }

  @override
  void attach(PipelineOwner owner) {
    super.attach(owner);
    _repaint.addListener(markNeedsCompositedLayerUpdate);
  }

  @override
  void detach() {
    _repaint.removeListener(markNeedsCompositedLayerUpdate);
    super.detach();
  }

  @override
  bool get isRepaintBoundary => true;

  @override
  bool get alwaysNeedsCompositing => true;

  @override
  OffsetLayer updateCompositedLayer({
    required covariant ShaderSamplerLayer? oldLayer,
  }) {
    final layer = oldLayer ?? ShaderSamplerLayer();
    layer
      ..builder = _builder
      ..size = size
      ..devicePixelRatio = _devicePixelRatio
      ..redraw();
    return layer;
  }

  @override
  void paint(PaintingContext context, Offset offset) {
    if (size.isEmpty) return;
    super.paint(context, offset);
    (layer as ShaderSamplerLayer?)?.childPainted();
  }
}

/// Draws a snapshot of its children through [builder] in their place.
class ShaderSamplerLayer extends OffsetLayer {
  ui.Picture? _lastPicture;
  ui.Image? _snapshot;
  Size _snapshotSize = Size.zero;
  double _snapshotPixelRatio = 0;
  bool _childPainted = true;

  ShaderSamplerBuilder? builder;
  Size size = Size.zero;
  double devicePixelRatio = 1.0;

  /// Draws again at the next frame, even if nothing below changed.
  void redraw() => markNeedsAddToScene();

  /// The children painted again, so the next drawing takes a new snapshot.
  void childPainted() => _childPainted = true;

  /// Whether the children are pictures alone. Then they change only when the
  /// sampler paints them again; a layer of their own can change by itself.
  bool get _childrenArePictures {
    for (Layer? child = firstChild; child != null; child = child.nextSibling) {
      if (child is! PictureLayer) return false;
    }
    return true;
  }

  ui.Image _buildChildScene(Rect bounds, double pixelRatio) {
    final sceneBuilder = ui.SceneBuilder();
    final transform = Matrix4.diagonal3Values(pixelRatio, pixelRatio, 1);
    sceneBuilder.pushTransform(transform.storage);
    addChildrenToScene(sceneBuilder);
    sceneBuilder.pop();
    return sceneBuilder.build().toImageSync(
      (pixelRatio * bounds.width).ceil(),
      (pixelRatio * bounds.height).ceil(),
    );
  }

  @override
  void dispose() {
    _lastPicture?.dispose();
    _snapshot?.dispose();
    super.dispose();
  }

  @override
  void addToScene(ui.SceneBuilder builder) {
    final draw = this.builder;
    if (size.isEmpty || draw == null) return;
    var snapshot = _snapshot;
    if (snapshot == null ||
        _childPainted ||
        !_childrenArePictures ||
        _snapshotSize != size ||
        _snapshotPixelRatio != devicePixelRatio) {
      snapshot?.dispose();
      snapshot = _snapshot = _buildChildScene(offset & size, devicePixelRatio);
      _snapshotSize = size;
      _snapshotPixelRatio = devicePixelRatio;
      _childPainted = false;
    }
    final recorder = ui.PictureRecorder();
    draw(snapshot, size, Canvas(recorder));
    final picture = recorder.endRecording();
    _lastPicture?.dispose();
    _lastPicture = picture;
    builder.addPicture(offset, picture);
  }
}
