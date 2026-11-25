import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations_en.dart';
import '../providers/library_share_sync.dart';
import '../utils/share_sync_messages.dart';

void main() {
  test(
    'a share that names another library shows its message until it clears',
    () async {
      final s = AppLocalizationsEn();
      final updates = StreamController<LibrarySyncStatusUpdated>();
      final shown = <String?>[];
      final notice = LibraryShareSyncNotice(
        updates: updates.stream,
        showNotice: (failure) => shown.add(shareSyncFailureMessage(s, failure)),
      );
      for (final failure in [
        ShareSyncFailure.shareChanged,
        ShareSyncFailure.shareChanged,
        null,
        ShareSyncFailure.shareChanged,
      ]) {
        updates.add(
          LibrarySyncStatusUpdated(
            libraryId: 'library-a',
            shareFailure: failure,
          ),
        );
      }
      await pumpEventQueue();
      expect(shown, [s.libraryInvalidSource, s.libraryInvalidSource]);
      notice.dispose();
      await updates.close();
    },
  );

  test(
    'an unavailable store shows a warning once and again after it clears',
    () async {
      final s = AppLocalizationsEn();
      final updates = StreamController<LibrarySyncStatusUpdated>();
      final shown = <String>[];
      final notice = LibraryShareSyncNotice(
        updates: updates.stream,
        showNotice: (failure) => shown.add(shareSyncFailureMessage(s, failure)),
      );
      for (final failure in [
        ShareSyncFailure.storeUnavailable,
        ShareSyncFailure.storeUnavailable,
        null,
        ShareSyncFailure.storeUnavailable,
      ]) {
        updates.add(
          LibrarySyncStatusUpdated(
            libraryId: 'library-a',
            shareFailure: failure,
          ),
        );
      }
      await pumpEventQueue();
      expect(shown, [
        s.libraryShareStoreUnavailable,
        s.libraryShareStoreUnavailable,
      ]);
      notice.dispose();
      await updates.close();
    },
  );

  test('an unavailable store maps to a warning, not silence', () {
    final s = AppLocalizationsEn();
    expect(
      shareSyncFailureMessage(s, ShareSyncFailure.storeUnavailable),
      s.libraryShareStoreUnavailable,
    );
    expect(
      shareSyncFailureMessage(s, ShareSyncFailure.readOnlyShare),
      s.libraryOpenDalReadOnly,
    );
  });
}
