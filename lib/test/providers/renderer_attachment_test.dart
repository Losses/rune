import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../providers/renderer_attachment.dart';

void main() {
  test('tracks renderer attachment changes and detachment', () async {
    final changes = StreamController<RendererAttachmentChanged>();
    final provider = RendererAttachmentProvider(
      initial: const RendererAttachmentChanged(
        renderer: AttachedRenderer(fingerprint: 'fp-b', alias: 'Bedroom'),
      ),
      changes: changes.stream,
    );
    expect(provider.renderer?.alias, 'Bedroom');

    changes.add(const RendererAttachmentChanged());
    await Future<void>.delayed(Duration.zero);
    expect(provider.renderer, isNull);

    provider.dispose();
    await changes.close();
  });
}
