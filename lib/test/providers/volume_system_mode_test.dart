import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../providers/renderer_attachment.dart';
import '../../providers/volume.dart';
import '../../utils/system_volume.dart';

class _FakeSystemVolume implements SystemVolume {
  SystemVolumeState? state = const SystemVolumeState(
    volume: 0.4,
    settable: true,
  );
  final setCalls = <double>[];
  final controller = StreamController<SystemVolumeState>.broadcast();

  /// What `read` waits for, when set.
  Future<SystemVolumeState?>? pending;

  @override
  Future<SystemVolumeState?> read() async => pending ?? state;

  /// Whether the runner does what it is asked.
  bool works = true;

  @override
  Future<bool> set(double volume) async {
    setCalls.add(volume);
    return works;
  }

  @override
  Stream<SystemVolumeState> get changes => controller.stream;
}

class _Rig {
  _Rig({TargetPlatform platform = TargetPlatform.macOS}) {
    provider = VolumeProvider(
      readStoredVolume: () async => null,
      writeStoredVolume: (volume) async => saved.add(volume),
      sendVolumeRequest: sent.add,
      systemVolume: system,
      renderers: renderers,
      hosted: HostedSignal(latest: false, changes: hosts.stream),
      platform: platform,
      clock: () => now,
    );
  }

  final system = _FakeSystemVolume();
  final hosts = StreamController<bool>();
  var now = DateTime(2026);
  final attachments = StreamController<RendererAttachmentChanged>();
  late final renderers = RendererAttachmentProvider(
    initial: const RendererAttachmentChanged(),
    changes: attachments.stream,
  );
  final sent = <VolumeRequest>[];
  final saved = <double>[];
  late final VolumeProvider provider;

  Future<void> host(String? hostedBy) async {
    hosts.add(hostedBy != null);
    await pump();
  }

  Future<void> pump() => Future<void>.delayed(Duration.zero);

  Future<void> dispose() async {
    provider.dispose();
    renderers.dispose();
    // Closing waits for a listener that a provider of another platform never had.
    unawaited(hosts.close());
    unawaited(attachments.close());
  }
}

void main() {
  test('follows the system volume while a host plays, and Rune after', () async {
    final rig = _Rig();
    addTearDown(rig.dispose);
    rig.provider.updateVolume(0.8);
    await Future<void>.delayed(const Duration(milliseconds: 250));
    expect(rig.provider.volume, 0.8);
    expect(rig.sent.map((request) => request.volume), [0.8]);
    expect(rig.saved, [0.8]);

    await rig.host('host');
    expect(rig.provider.usesSystemVolume, isTrue);
    expect(rig.provider.volume, 0.4);

    // A hardware key moves it.
    rig.system.controller.add(
      const SystemVolumeState(volume: 0.55, settable: true),
    );
    await rig.pump();
    expect(rig.provider.volume, 0.55);

    // Changes go to the system only: no request to the hub, nothing saved.
    rig.provider.updateVolume(0.2);
    await Future<void>.delayed(const Duration(milliseconds: 250));
    expect(rig.system.setCalls, [0.2]);
    expect(rig.provider.volume, 0.2);
    expect(rig.sent.map((request) => request.volume), [0.8]);
    expect(rig.saved, [0.8]);

    // Rune's own volume is back, and the system volume stays where it was put.
    await rig.host(null);
    expect(rig.provider.usesSystemVolume, isFalse);
    expect(rig.provider.volume, 0.8);
    rig.system.controller.add(
      const SystemVolumeState(volume: 0.9, settable: true),
    );
    await rig.pump();
    expect(rig.provider.volume, 0.8);
    expect(rig.system.setCalls, [0.2]);
  });

  test(
    'an attached renderer plays the track, so Rune keeps its volume',
    () async {
      final rig = _Rig();
      addTearDown(rig.dispose);
      await rig.host('host');
      expect(rig.provider.usesSystemVolume, isTrue);

      rig.attachments.add(
        const RendererAttachmentChanged(
          renderer: AttachedRenderer(fingerprint: 'fp', alias: 'Bedroom'),
        ),
      );
      await rig.pump();
      expect(rig.provider.usesSystemVolume, isFalse);

      rig.attachments.add(const RendererAttachmentChanged());
      await rig.pump();
      expect(rig.provider.usesSystemVolume, isTrue);
    },
  );

  test('an output without a volume to move disables the control', () async {
    final rig = _Rig();
    addTearDown(rig.dispose);
    rig.system.state = const SystemVolumeState(volume: 1, settable: false);
    await rig.host('host');
    expect(rig.provider.adjustable, isFalse);

    rig.provider.updateVolume(0.3);
    expect(rig.system.setCalls, isEmpty);

    // Another output, with a volume.
    rig.system.controller.add(
      const SystemVolumeState(volume: 0.6, settable: true),
    );
    await rig.pump();
    expect(rig.provider.adjustable, isTrue);
    expect(rig.provider.volume, 0.6);
  });

  test(
    'iOS shows the system volume view, which sets the volume itself',
    () async {
      final rig = _Rig(platform: TargetPlatform.iOS);
      addTearDown(rig.dispose);
      expect(rig.provider.showsSystemVolumeView, isFalse);
      await rig.host('host');
      expect(rig.provider.showsSystemVolumeView, isTrue);
      expect(rig.provider.volume, 0.4);

      rig.provider.updateVolume(0.1);
      expect(rig.system.setCalls, isEmpty);
      expect(rig.sent, isEmpty);
    },
  );

  test(
    'Android sets the system volume with the slider, without a view',
    () async {
      final rig = _Rig(platform: TargetPlatform.android);
      addTearDown(rig.dispose);
      await rig.host('host');
      expect(rig.provider.usesSystemVolume, isTrue);
      expect(rig.provider.showsSystemVolumeView, isFalse);
      expect(rig.provider.adjustable, isTrue);
      expect(rig.provider.volume, 0.4);

      rig.provider.updateVolume(0.8);
      await rig.pump();
      expect(rig.system.setCalls, [0.8]);
      expect(rig.sent, isEmpty);
    },
  );

  test('starts in system mode when the host already plays', () async {
    final system = _FakeSystemVolume();
    final provider = VolumeProvider(
      readStoredVolume: () async => null,
      writeStoredVolume: (_) async {},
      sendVolumeRequest: (_) {},
      systemVolume: system,
      hosted: const HostedSignal(latest: true, changes: Stream.empty()),
      platform: TargetPlatform.macOS,
    );
    addTearDown(provider.dispose);
    await Future<void>.delayed(Duration.zero);
    expect(provider.usesSystemVolume, isTrue);
    expect(provider.volume, 0.4);
  });

  test(
    'the system reports right after a set are the set\'s echo, not news',
    () async {
      final rig = _Rig();
      addTearDown(rig.dispose);
      await rig.host('host');

      rig.provider.updateVolume(0.2);
      rig.provider.updateVolume(0.3);
      await rig.pump();
      // The report of the first set lands after the second.
      rig.system.controller.add(
        const SystemVolumeState(volume: 0.2, settable: true),
      );
      await rig.pump();
      expect(rig.provider.volume, 0.3);

      // Later, a hardware key is news again.
      rig.now = rig.now.add(VolumeProvider.systemEchoWindow);
      rig.system.controller.add(
        const SystemVolumeState(volume: 0.7, settable: true),
      );
      await rig.pump();
      expect(rig.provider.volume, 0.7);
    },
  );

  test('a report inside the window shows once the window closes', () async {
    final rig = _Rig();
    addTearDown(rig.dispose);
    await rig.host('host');

    rig.provider.updateVolume(0.2);
    // A volume key, or the device rounding the final value.
    rig.system.controller.add(
      const SystemVolumeState(volume: 0.25, settable: true),
    );
    await rig.pump();
    expect(rig.provider.volume, 0.2);

    rig.now = rig.now.add(VolumeProvider.systemEchoWindow);
    await Future<void>.delayed(
      VolumeProvider.systemEchoWindow + const Duration(milliseconds: 50),
    );
    expect(rig.provider.volume, 0.25);

    // A newer set of Rune's own wins over a report held back for it.
    rig.provider.updateVolume(0.5);
    rig.system.controller.add(
      const SystemVolumeState(volume: 0.45, settable: true),
    );
    await rig.pump();
    rig.provider.updateVolume(0.6);
    rig.now = rig.now.add(VolumeProvider.systemEchoWindow);
    await Future<void>.delayed(
      VolumeProvider.systemEchoWindow + const Duration(milliseconds: 50),
    );
    expect(rig.provider.volume, 0.6);
  });

  test('a report after the window outweighs one held back in it', () async {
    final rig = _Rig();
    addTearDown(rig.dispose);
    await rig.host('host');

    rig.provider.updateVolume(0.5);
    rig.system.controller.add(
      const SystemVolumeState(volume: 0.2, settable: true),
    );
    await rig.pump();
    expect(rig.provider.volume, 0.5);

    // The window closes, and a newer report lands before the held one is applied.
    rig.now = rig.now.add(VolumeProvider.systemEchoWindow);
    rig.system.controller.add(
      const SystemVolumeState(volume: 0.7, settable: true),
    );
    await rig.pump();
    expect(rig.provider.volume, 0.7);
    await Future<void>.delayed(
      VolumeProvider.systemEchoWindow + const Duration(milliseconds: 50),
    );
    expect(rig.provider.volume, 0.7);
  });

  test(
    'the volume is unknown, and the control disabled, until the system answers',
    () async {
      final rig = _Rig();
      addTearDown(rig.dispose);
      final answer = Completer<SystemVolumeState?>();
      rig.system.pending = answer.future;
      rig.provider.updateVolume(0.8);
      await rig.host('host');
      expect(rig.provider.usesSystemVolume, isTrue);
      expect(rig.provider.volume, isNot(0.8));
      expect(rig.provider.adjustable, isFalse);

      answer.complete(const SystemVolumeState(volume: 0.5, settable: true));
      await rig.pump();
      expect(rig.provider.volume, 0.5);
      expect(rig.provider.adjustable, isTrue);
    },
  );

  test(
    'a set the runner could not do is taken back to what the system says',
    () async {
      final rig = _Rig();
      addTearDown(rig.dispose);
      await rig.host('host');
      rig.system.works = false;

      rig.provider.updateVolume(0.9);
      expect(rig.provider.volume, 0.9);
      await rig.pump();
      await rig.pump();
      expect(rig.system.setCalls, [0.9]);
      expect(rig.provider.volume, 0.4);
    },
  );

  test('other platforms never use the system volume', () async {
    for (final platform in [TargetPlatform.windows, TargetPlatform.linux]) {
      final rig = _Rig(platform: platform);
      await rig.host('host');
      expect(rig.provider.usesSystemVolume, isFalse, reason: platform.name);
      rig.provider.updateVolume(0.6);
      await Future<void>.delayed(const Duration(milliseconds: 250));
      expect(rig.provider.volume, 0.6);
      expect(rig.system.setCalls, isEmpty);
      expect(rig.sent.map((request) => request.volume), [0.6]);
      await rig.dispose();
    }
  });

  test(
    'a pending change of Rune\'s volume goes out, whatever the system mode does',
    () async {
      final rig = _Rig();
      addTearDown(rig.dispose);
      rig.provider.updateVolume(0.7);
      await rig.host('host');
      rig.provider.updateVolume(0.2);
      await Future<void>.delayed(const Duration(milliseconds: 250));
      expect(rig.sent.map((request) => request.volume), [0.7]);
      expect(rig.system.setCalls, [0.2]);
      expect(rig.saved, [0.7]);
    },
  );
}
