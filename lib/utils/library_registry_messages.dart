import '../bindings/bindings.dart';
import '../l10n/app_localizations.dart';

String libraryRegistryFailureMessage(
  AppLocalizations s,
  Object kind,
) => switch (kind) {
  OpenLibraryFailureKind.recordNotFound ||
  RemoveLibraryRecordFailureKind.recordNotFound => s.libraryRecordNotFound,
  RemoveLibraryRecordFailureKind.recordIsOpen => s.libraryRecordIsOpen,
  OpenLibraryFailureKind.mediaFolderMissing => s.libraryMediaFolderMissing,
  RegisterLibraryFailureKind.untrusted ||
  RegisterLibraryFailureKind.remoteUntrusted ||
  OpenLibraryFailureKind.remoteUntrusted => s.libraryUntrusted,
  RegisterLibraryFailureKind.invalidSource => s.libraryInvalidSource,
  RegisterLibraryFailureKind.storage ||
  OpenLibraryFailureKind.storage => s.libraryStorageFailure,
  RegisterLibraryFailureKind.remoteNotAuthorized ||
  OpenLibraryFailureKind.remoteNotAuthorized => s.libraryRemoteNotAuthorized,
  RegisterLibraryFailureKind.remoteUnreachable ||
  OpenLibraryFailureKind.remoteUnreachable => s.libraryRemoteUnreachable,
  RegisterLibraryFailureKind.openDalAuthenticationRejected =>
    s.libraryOpenDalAuthenticationRejected,
  RegisterLibraryFailureKind.subsonicWrongCredentials =>
    s.libraryOpenDalAuthenticationRejected,
  RegisterLibraryFailureKind.subsonicUnreachable => s.libraryOpenDalUnreachable,
  RegisterLibraryFailureKind.subsonicServerTooOld ||
  RegisterLibraryFailureKind.subsonicClientTooOld =>
    s.libraryProtocolIncompatible,
  RegisterLibraryFailureKind.subsonicRoleMissing =>
    s.capabilityAccountRoleMissing,
  RegisterLibraryFailureKind.subsonicNotSubsonic => s.libraryInvalidSource,
  RegisterLibraryFailureKind.openDalUnreachable => s.libraryOpenDalUnreachable,
  RegisterLibraryFailureKind.openDalNotFolder => s.libraryOpenDalNotFolder,
  RegisterLibraryFailureKind.openDalReadOnly => s.libraryOpenDalReadOnly,
  RegisterLibraryFailureKind.openDalShareInvalid => s.libraryStorageFailure,
  RegisterLibraryFailureKind.openDalShareChanged => s.libraryInvalidSource,
  RegisterLibraryFailureKind.remoteProtocol ||
  OpenLibraryFailureKind.remoteProtocol => s.libraryRemoteProtocolFailure,
  OpenLibraryFailureKind.protocolIncompatible => s.libraryProtocolIncompatible,
  OpenLibraryFailureKind.notSupportedYet => s.libraryNotSupportedYet,
  _ => s.unknownError,
};
