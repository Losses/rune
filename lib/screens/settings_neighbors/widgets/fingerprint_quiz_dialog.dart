import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/api/fetch_server_certificate.dart';
import '../../../utils/dialogs/information/information.dart';
import '../../../utils/l10n.dart';
import '../../../widgets/fingerprint/fingerprint_quiz.dart';
import '../../../widgets/no_shortcuts.dart';

class FingerprintQuizDialog extends StatefulWidget {
  const FingerprintQuizDialog(
      {super.key, required this.$close, required this.host});

  final String host;
  final void Function(void) $close;

  @override
  FingerprintQuizDialogState createState() => FingerprintQuizDialogState();
}

class FingerprintQuizDialogState extends State<FingerprintQuizDialog> {
  late Future<String> _fingerprintFuture;

  @override
  void initState() {
    super.initState();
    _fingerprintFuture = fetchServerCertificate(widget.host);
  }

  void _handleAnswer(bool correct) {
    widget.$close(null);

    final s = S.of(context);

    if (!correct) {
      showInformationDialog(
        context: context,
        title: s.pairingFailureTitle,
        subtitle: s.pairingFailureMessage,
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return NoShortcuts(
      ContentDialog(
        title: Column(
          children: [
            SizedBox(height: 8),
            Text(S.of(context).pairingTitle),
          ],
        ),
        content: FutureBuilder<String>(
          future: _fingerprintFuture,
          builder: (context, snapshot) {
            if (snapshot.connectionState == ConnectionState.waiting) {
              return Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const ProgressRing(),
                  const SizedBox(height: 16),
                  Text(s.pairingLoading),
                ],
              );
            }
            if (snapshot.hasError) {
              return Text(
                'Error: ${snapshot.error}',
                style: TextStyle(height: 1.4),
              );
            }
            return Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(
                  s.pairingInstructions,
                  style: TextStyle(height: 1.4),
                ),
                const SizedBox(height: 24),
                FingerprintQuiz(
                  fingerprint: snapshot.data!,
                  onAnswer: _handleAnswer,
                )
              ],
            );
          },
        ),
        actions: [
          Button(
            onPressed: () => widget.$close(null),
            child: Text(s.cancel),
          ),
        ],
      ),
    );
  }
}
