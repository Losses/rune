import '../bindings/bindings.dart';
import '../l10n/app_localizations.dart';

/// The message for a library whose share sync stopped.
String shareSyncFailureMessage(AppLocalizations s, ShareSyncFailure failure) =>
    switch (failure) {
      ShareSyncFailure.shareChanged => s.libraryShareIdentityChanged,
      ShareSyncFailure.readOnlyShare => s.libraryOpenDalReadOnly,
      ShareSyncFailure.credentialsRejected =>
        s.libraryOpenDalAuthenticationRejected,
      ShareSyncFailure.objectMissing => s.libraryOpenDalNotFolder,
      ShareSyncFailure.probeFailed => s.libraryStorageFailure,
      ShareSyncFailure.storeUnavailable => s.libraryShareStoreUnavailable,
    };

/// The message for a sync the user asked for that failed. Nothing retries such
/// a sync, so it must not promise a retry.
String requestedShareSyncFailureMessage(
  AppLocalizations s,
  ShareSyncFailure failure,
) => switch (failure) {
  ShareSyncFailure.storeUnavailable => s.libraryShareStoreUnavailableOnRequest,
  _ => shareSyncFailureMessage(s, failure),
};
