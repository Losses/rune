import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations_en.dart';
import '../providers/crash.dart';
import '../providers/remote_connection_loss.dart';
import '../utils/remote_connection_messages.dart';

void main() {
  test('remote loss kinds map to localized messages', () {
    final s = AppLocalizationsEn();
    expect(remoteLossMessage(s, RemoteLossKind.revoked), s.remoteLossRevoked);
    expect(remoteLossMessage(s, RemoteLossKind.detached), s.remoteLossDetached);
    expect(
      remoteLossMessage(s, RemoteLossKind.serverStopped),
      s.remoteLossServerStopped,
    );
    expect(
      remoteLossMessage(s, RemoteLossKind.networkLost),
      s.remoteLossNetworkLost,
    );
  });

  test('remote open error kinds map to localized messages', () {
    final s = AppLocalizationsEn();
    expect(
      remoteOpenErrorMessage(s, LibraryOpenErrorKind.remoteUntrusted),
      s.remoteOpenUntrusted,
    );
    expect(
      remoteOpenErrorMessage(s, LibraryOpenErrorKind.remoteNotAuthorized),
      s.remoteOpenNotAuthorized,
    );
    expect(
      remoteOpenErrorMessage(s, LibraryOpenErrorKind.remoteUnreachable),
      s.remoteOpenUnreachable,
    );
    expect(
      remoteOpenErrorMessage(s, LibraryOpenErrorKind.remoteProtocol),
      s.remoteOpenProtocol,
    );
    expect(remoteOpenErrorMessage(s, null), s.unknownError);
  });

  test('loss closes the library without touching crash state', () async {
    final losses = StreamController<RemoteConnectionLost>();
    final crash = CrashProvider();
    final closed = Completer<void>();
    final shown = Completer<RemoteLossKind>();
    final handler = RemoteConnectionLossHandler(
      losses: losses.stream,
      closeLibrary: () async => closed.complete(),
      showLoss: (kind) {
        expect(closed.isCompleted, isTrue);
        shown.complete(kind);
      },
    );
    losses.add(
      const RemoteConnectionLost(
        kind: RemoteLossKind.revoked,
        detail: 'revoked',
      ),
    );
    expect(await shown.future, RemoteLossKind.revoked);
    expect(crash.report, isNull);
    handler.dispose();
    crash.dispose();
    await losses.close();
  });
}
