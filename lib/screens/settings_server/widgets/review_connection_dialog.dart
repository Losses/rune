import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/fingerprint_figure.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../providers/broadcast.dart';
import '../../../widgets/subtitle_button.dart';

class ReviewConnectionDialog extends StatelessWidget {
  final void Function(void) $close;
  final String fingerprint;

  const ReviewConnectionDialog({
    super.key,
    required this.$close,
    required this.fingerprint,
  });

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
              s.connectionVerificationMessage,
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
                        onPressed: () => {},
                      ),
                      const SizedBox(height: 8),
                      SubtitleButton(
                        title: s.blockDevice,
                        subtitle: s.blockDeviceSubtitle,
                        onPressed: () => {},
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ],
        ),
        actions: [
          Button(
            onPressed: () => $close(null),
            child: Text(s.cancel),
          ),
        ],
      ),
    );
  }
}
