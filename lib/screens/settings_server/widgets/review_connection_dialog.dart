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
        title: Column(
          children: [
            SizedBox(height: 8),
            Text(s.reviewRequestTitle),
          ],
        ),
        content: Column(
          children: [
            Text(
              s.connectionVerificationMessage,
              style: TextStyle(height: 1.4),
            ),
            Row(
              children: [
                InfoLabel(
                  label: s.deviceFingerprint,
                  child: FingerprintFigure(fingerprint: fingerprint),
                ),
                const SizedBox(width: 16),
                Column(
                  children: [
                    SubtitleButton(
                      title: s.allowAccess,
                      subtitle: s.allowAccessSubtitle,
                      onPressed: () => {},
                    ),
                    SubtitleButton(
                      title: s.blockDevice,
                      subtitle: s.blockDeviceSubtitle,
                      onPressed: () => {},
                    ),
                  ],
                )
              ],
            )
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
