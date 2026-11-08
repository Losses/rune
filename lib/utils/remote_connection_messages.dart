import '../bindings/bindings.dart';
import '../l10n/app_localizations.dart';

String remoteOpenErrorMessage(AppLocalizations s, LibraryOpenErrorKind? kind) {
  return switch (kind) {
    LibraryOpenErrorKind.remoteUntrusted => s.remoteOpenUntrusted,
    LibraryOpenErrorKind.remoteNotAuthorized => s.remoteOpenNotAuthorized,
    LibraryOpenErrorKind.remoteUnreachable => s.remoteOpenUnreachable,
    LibraryOpenErrorKind.remoteProtocol => s.remoteOpenProtocol,
    // No kind: Rust did not answer in time or panicked; nothing more specific is known.
    null => s.unknownError,
  };
}

String remoteLossMessage(AppLocalizations s, RemoteLossKind kind) {
  return switch (kind) {
    RemoteLossKind.revoked => s.remoteLossRevoked,
    RemoteLossKind.serverStopped => s.remoteLossServerStopped,
    RemoteLossKind.networkLost => s.remoteLossNetworkLost,
    RemoteLossKind.detached => s.remoteLossDetached,
    RemoteLossKind.protocolIncompatible => 'A protocol update is required to reconnect.', // R10b
  };
}
