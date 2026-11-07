import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations_en.dart';
import '../providers/renderer_link_loss.dart';
import '../utils/remote_connection_messages.dart';

void main() {
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

  test(
    'renderer link loss shows its alias and keeps the library open',
    () async {
      final losses = StreamController<RendererLinkLost>();
      final noticed = Completer<String>();
      final handler = RendererLinkLossHandler(
        losses: losses.stream,
        showNotice: (alias) =>
            noticed.complete(AppLocalizationsEn().rendererLinkLost(alias)),
      );
      losses.add(const RendererLinkLost(fingerprint: 'fp-b', alias: 'Bedroom'));
      expect(
        await noticed.future.timeout(const Duration(seconds: 1)),
        'Connection to Bedroom lost',
      );
      handler.dispose();
      await losses.close();
    },
  );
}
