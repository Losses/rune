import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/utils/api/set_sync_settings.dart';

const _settings = GetSyncSettingsResponse(
  requestId: 'loaded',
  mediaServer: true,
  broadcast: false,
  acceptRemoteControl: true,
  acceptSync: false,
  autoDeviceSync: true,
  autoShareSync: false,
  missingHostFile: MissingHostFilePolicy.keepAsRemote,
);

List<Object> _values(GetSyncSettingsResponse settings) => [
  settings.mediaServer,
  settings.broadcast,
  settings.acceptRemoteControl,
  settings.acceptSync,
  settings.autoDeviceSync,
  settings.autoShareSync,
  settings.missingHostFile,
];

void main() {
  test('a change with no field keeps every setting', () {
    expect(
      _values(const SyncSettingsChange().applyTo(_settings)),
      _values(_settings),
    );
  });

  test('each field of a change replaces its setting and keeps the others', () {
    const kept = [
      true,
      false,
      true,
      false,
      true,
      false,
      MissingHostFilePolicy.keepAsRemote,
    ];
    const changes = <(SyncSettingsChange, Object)>[
      (SyncSettingsChange(mediaServer: false), false),
      (SyncSettingsChange(broadcast: true), true),
      (SyncSettingsChange(acceptRemoteControl: false), false),
      (SyncSettingsChange(acceptSync: true), true),
      (SyncSettingsChange(autoDeviceSync: false), false),
      (SyncSettingsChange(autoShareSync: true), true),
      (
        SyncSettingsChange(
          missingHostFile: MissingHostFilePolicy.removeEverywhere,
        ),
        MissingHostFilePolicy.removeEverywhere,
      ),
    ];

    for (final (index, (change, replaced)) in changes.indexed) {
      expect(_values(change.applyTo(_settings)), [
        ...kept.take(index),
        replaced,
        ...kept.skip(index + 1),
      ], reason: 'field $index');
    }
  });

  test('a change with several fields replaces all of them', () {
    final changed = const SyncSettingsChange(
      acceptSync: true,
      missingHostFile: MissingHostFilePolicy.removeEverywhere,
    ).applyTo(_settings);

    expect(_values(changed), [
      true,
      false,
      true,
      true,
      true,
      false,
      MissingHostFilePolicy.removeEverywhere,
    ]);
  });
}
