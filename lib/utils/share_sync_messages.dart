import '../bindings/bindings.dart';
import '../l10n/app_localizations.dart';

/// The message for a library whose share sync stopped, or null for an
/// unavailable store, which the share worker retries on its own.
String? shareSyncFailureMessage(AppLocalizations s, ShareSyncFailure failure) =>
    switch (failure) {
      ShareSyncFailure.shareChanged => s.libraryInvalidSource,
      ShareSyncFailure.readOnlyShare => s.libraryOpenDalReadOnly,
      ShareSyncFailure.credentialsRejected =>
        s.libraryOpenDalAuthenticationRejected,
      ShareSyncFailure.objectMissing => s.libraryOpenDalNotFolder,
      ShareSyncFailure.probeFailed => s.libraryStorageFailure,
      ShareSyncFailure.storeUnavailable => null,
    };
