import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations_en.dart';
import '../providers/remote_library_reconnect.dart';
import '../utils/api/request_failure.dart';

void main() {
  test(
    'reconnect notice clears on recovery, loss, library exit and disposal',
    () async {
      final reconnecting =
          StreamController<RemoteLibraryReconnecting>.broadcast(sync: true);
      final reconnected = StreamController<RemoteLibraryReconnected>.broadcast(
        sync: true,
      );
      final losses = StreamController<RemoteConnectionLost>.broadcast(
        sync: true,
      );
      final notice = RemoteLibraryReconnectNotice(
        reconnecting: reconnecting.stream,
        reconnected: reconnected.stream,
        losses: losses.stream,
      );
      void disconnect() {
        reconnecting.add(
          const RemoteLibraryReconnecting(fingerprint: 'b', alias: 'Bedroom'),
        );
        expect(notice.value, 'Bedroom');
      }

      expect(notice.value, isNull);
      disconnect();
      reconnected.add(
        const RemoteLibraryReconnected(fingerprint: 'old', alias: 'Old'),
      );
      expect(notice.value, 'Bedroom');
      reconnected.add(
        const RemoteLibraryReconnected(fingerprint: 'b', alias: 'Bedroom'),
      );
      expect(notice.value, isNull);
      disconnect();
      losses.add(
        const RemoteConnectionLost(
          kind: RemoteLossKind.networkLost,
          detail: 'deadline',
        ),
      );
      expect(notice.value, isNull);
      disconnect();
      notice.clear();
      expect(notice.value, isNull);
      notice.dispose();
      expect(reconnecting.hasListener, isFalse);
      expect(reconnected.hasListener, isFalse);
      expect(losses.hasListener, isFalse);
      await reconnecting.close();
      await reconnected.close();
      await losses.close();
    },
  );

  test('reconnecting request failure names the device using localization', () {
    expect(
      requestFailureMessage(
        AppLocalizationsEn(),
        const RequestFailure(
          'FetchAllPlaylistsRequest',
          RequestErrorKind.unavailable,
          'diagnostic',
          RequestFailureCauseReconnecting(alias: 'Bedroom'),
        ),
      ),
      'Reconnecting to Bedroom…',
    );
  });
}
