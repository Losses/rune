import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/subtitle_button.dart';
import '../../../widgets/fingerprint/fingerprint_figure.dart';
import '../../../bindings/bindings.dart';
import '../../../providers/broadcast.dart';

class ReviewConnectionDialog extends StatefulWidget {
  final void Function(ClientStatus?) $close;
  final ClientSummary clientSummary;

  const ReviewConnectionDialog({
    super.key,
    required this.$close,
    required this.clientSummary,
  });

  @override
  State<ReviewConnectionDialog> createState() => _ReviewConnectionDialogState();
}

class _ReviewConnectionDialogState extends State<ReviewConnectionDialog> {
  late ClientStatus status;

  @override
  void initState() {
    status = widget.clientSummary.status;

    super.initState();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final broadcast = Provider.of<BroadcastProvider>(context);
    final fingerprint = broadcast.fingerprint;

    return NoShortcuts(
      ContentDialog(
        constraints: BoxConstraints(maxWidth: 520),
        title: Column(
          children: [
            SizedBox(height: 8),
            Text(s.reviewRequestTitle),
          ],
        ),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              s.connectionVerificationMessage(
                widget.clientSummary.alias,
                widget.clientSummary.deviceModel,
              ),
              style: TextStyle(height: 1.4),
            ),
            const SizedBox(height: 16),
            Row(
              children: [
                Flexible(
                  child: Container(
                    constraints: BoxConstraints(maxWidth: 226),
                    child: FingerprintFigure(
                      fingerprint: fingerprint,
                      buttonStyle: const ButtonStyle(
                        padding: WidgetStatePropertyAll(EdgeInsets.all(0)),
                      ),
                      textStyle: const TextStyle(
                        fontFamily: 'NotoRunic',
                        fontSize: 14,
                        letterSpacing: 4,
                      ),
                    ),
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    children: [
                      SubtitleButton(
                        title: s.allowAccess,
                        subtitle: s.allowAccessSubtitle,
                        selected: status == ClientStatus.approved,
                        onPressed: () => setState(() {
                          status = ClientStatus.approved;
                        }),
                      ),
                      const SizedBox(height: 8),
                      SubtitleButton(
                        title: s.blockDevice,
                        subtitle: s.blockDeviceSubtitle,
                        selected: status != ClientStatus.approved,
                        onPressed: () => setState(() {
                          status = ClientStatus.blocked;
                        }),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ],
        ),
        actions: [
          FilledButton(
            onPressed: () => widget.$close(status),
            child: Text(S.of(context).confirm),
          ),
          Button(
            onPressed: () => widget.$close(null),
            child: Text(s.cancel),
          ),
        ],
      ),
    );
  }
}
