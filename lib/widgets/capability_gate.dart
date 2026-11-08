import 'package:fluent_ui/fluent_ui.dart';

import '../bindings/bindings.dart';
import '../utils/l10n.dart';

String capabilityReasonText(BuildContext context, CapabilityReason reason) {
  final strings = S.of(context);
  return switch (reason) {
    CapabilityReason.notImplemented => strings.capabilityNotImplemented,
    CapabilityReason.ownerOnly => strings.capabilityOwnerOnly,
    CapabilityReason.peerTooOld => 'The peer needs a protocol update.', // R10b
  };
}

class CapabilityGate extends StatelessWidget {
  const CapabilityGate({
    super.key,
    required this.capability,
    required this.child,
  });

  final RequestCapability capability;
  final Widget child;

  @override
  Widget build(BuildContext context) => switch (capability) {
    RequestCapabilityAvailable() => child,
    RequestCapabilityHidden() => const SizedBox.shrink(),
    RequestCapabilityDisabled(:final reason) => Tooltip(
      message: capabilityReasonText(context, reason),
      child: Semantics(
        enabled: false,
        child: Opacity(opacity: 0.45, child: IgnorePointer(child: child)),
      ),
    ),
    _ => child,
  };
}
