import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';

({String subtitle, bool showBadge}) deviceStatusPresentation(
  ClientStatus status,
  AppLocalizations strings,
) => switch (status) {
  ClientStatus.approved => (subtitle: strings.myDevice, showBadge: true),
  ClientStatus.pending => (
    subtitle: strings.waitingForApproval,
    showBadge: true,
  ),
  ClientStatus.blocked => (subtitle: strings.blockedStatus, showBadge: true),
};
