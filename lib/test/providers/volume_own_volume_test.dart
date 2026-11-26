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
  var reads = 0;
  final setCalls = <double>[];
  final controller = StreamController<SystemVolumeState>.broadcast();

  @override
  Future<SystemVolumeState?> read() async {
    reads++;
    return state;
  }

  @override
  Future<bool> set(double volume) async {
    setCalls.add(volume);
    return true;
  }

  @override
  Stream<SystemVolumeState> get changes => controller.stream;
}

class _Rig {
  _Rig({
    TargetPlatform platform = TargetPlatform.macOS,
    PlayerHostChanged? latest,
  }) {
    provider = VolumeProvider(
      readStoredVolume: () async => null,
      writeStoredVolume: (volume) async => saved.add(volume),
      sendVolumeRequest: sent.add,
      systemVolume: system,
      renderers: renderers,
      hosted: HostedSignal.fromReports(latest: latest, changes: hosts.stream),
      platform: platform,
      clock: () => now,
    );
  }

  final system = _FakeSystemVolume();
  final hosts = StreamController<PlayerHostChanged>();
  var now = DateTime(2026);
  final attachments = StreamController<RendererAttachmentChanged>();
  late final renderers = RendererAttachmentProvider(
    initial: const RendererAttachmentChanged(),
    changes: attachments.stream,
  );
  final sent = <VolumeRequest>[];
  final saved = <double>[];
  late final VolumeProvider provider;

  /// A host plays this device's track, with a volume of its own or without one, or none plays.
  Future<void> host({required bool hosted, bool ownVolume = false}) async {
    hosts.add(PlayerHostChanged(hosted: hosted, ownVolume: ownVolume));
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
  test(
    'keeps Rune\'s volume while a host with a volume of its own plays',
    () async {
      final rig = _Rig();
      addTearDown(rig.dispose);
      rig.provider.updateVolume(0.8);
      await Future<void>.delayed(const Duration(milliseconds: 250));

      await rig.host(hosted: true, ownVolume: true);
      expect(rig.provider.usesSystemVolume, isFalse);
      expect(rig.provider.adjustable, isTrue);
      expect(rig.provider.volume, 0.8);

      // A hardware key moves the system volume, not Rune's.
      rig.system.controller.add(
        const SystemVolumeState(volume: 0.55, settable: true),
      );
      await rig.pump();
      expect(rig.provider.volume, 0.8);

      // Changes go to the hub, which passes them to the host, and are saved: the system
      // volume is not touched.
      rig.provider.updateVolume(0.2);
      await Future<void>.delayed(const Duration(milliseconds: 250));
      expect(rig.provider.volume, 0.2);
      expect(rig.sent.map((request) => request.volume), [0.8, 0.2]);
      expect(rig.saved, [0.8, 0.2]);
      expect(rig.system.setCalls, isEmpty);
      expect(rig.system.reads, 0);

      // Once the host lets go, nothing changes.
      await rig.host(hosted: false);
      expect(rig.provider.usesSystemVolume, isFalse);
      expect(rig.provider.volume, 0.2);
    },
  );

  test('a host without a volume of its own takes the system volume', () async {
    final rig = _Rig();
    addTearDown(rig.dispose);
    rig.provider.updateVolume(0.8);
    await Future<void>.delayed(const Duration(milliseconds: 250));

    await rig.host(hosted: true, ownVolume: true);
    expect(rig.provider.usesSystemVolume, isFalse);

    // The host starts again, and says it has none.
    await rig.host(hosted: true);
    expect(rig.provider.usesSystemVolume, isTrue);
    expect(rig.provider.volume, 0.4);

    // It has one again: Rune's volume is back, as it was.
    await rig.host(hosted: true, ownVolume: true);
    expect(rig.provider.usesSystemVolume, isFalse);
    expect(rig.provider.volume, 0.8);
  });

  test('starts with Rune\'s volume when such a host already plays', () async {
    final rig = _Rig(
      latest: const PlayerHostChanged(hosted: true, ownVolume: true),
    );
    addTearDown(rig.dispose);
    await rig.pump();
    expect(rig.provider.usesSystemVolume, isFalse);
    expect(rig.system.reads, 0);

    rig.provider.updateVolume(0.6);
    await Future<void>.delayed(const Duration(milliseconds: 250));
    expect(rig.provider.volume, 0.6);
    expect(rig.sent.map((request) => request.volume), [0.6]);
    expect(rig.system.setCalls, isEmpty);
  });

  test('starts in system mode when a host without one already plays', () async {
    final rig = _Rig(
      latest: const PlayerHostChanged(hosted: true, ownVolume: false),
    );
    addTearDown(rig.dispose);
    await rig.pump();
    expect(rig.provider.usesSystemVolume, isTrue);
    expect(rig.provider.volume, 0.4);
  });

  test('iOS keeps the slider for a host with a volume of its own', () async {
    final rig = _Rig(platform: TargetPlatform.iOS);
    addTearDown(rig.dispose);
    await rig.host(hosted: true, ownVolume: true);
    expect(rig.provider.showsSystemVolumeView, isFalse);
    expect(rig.provider.usesSystemVolume, isFalse);

    rig.provider.updateVolume(0.3);
    await Future<void>.delayed(const Duration(milliseconds: 250));
    expect(rig.sent.map((request) => request.volume), [0.3]);
    expect(rig.system.setCalls, isEmpty);
  });

  test('an attached renderer plays the track, whatever the host has', () async {
    final rig = _Rig();
    addTearDown(rig.dispose);
    await rig.host(hosted: true);
    expect(rig.provider.usesSystemVolume, isTrue);

    rig.attachments.add(
      const RendererAttachmentChanged(
        renderer: AttachedRenderer(fingerprint: 'fp', alias: 'Bedroom'),
      ),
    );
    await rig.pump();
    expect(rig.provider.usesSystemVolume, isFalse);

    await rig.host(hosted: true, ownVolume: true);
    rig.attachments.add(const RendererAttachmentChanged());
    await rig.pump();
    expect(rig.provider.usesSystemVolume, isFalse);
  });
}
