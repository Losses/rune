import 'dart:async';

import '../bindings/bindings.dart';

/// Tells the user once when a library's share sync stops on a failure. The share
/// worker reports the failure again on every retry, and reports none once a pass
/// succeeds, so a failure shows again only after it changed or cleared.
class LibraryShareSyncNotice {
  final void Function(ShareSyncFailure failure) showNotice;
  final Map<String, ShareSyncFailure?> _reported = {};
  late final StreamSubscription<LibrarySyncStatusUpdated> _subscription;

  LibraryShareSyncNotice({
    required Stream<LibrarySyncStatusUpdated> updates,
    required this.showNotice,
  }) {
    _subscription = updates.listen((update) {
      final failure = update.shareFailure;
      final previous = _reported[update.libraryId];
      _reported[update.libraryId] = failure;
      if (failure != null && failure != previous) showNotice(failure);
    });
  }

  void dispose() => _subscription.cancel();
}
