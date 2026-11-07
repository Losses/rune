import '../../../utils/api/request_failure.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/api/fetch_server_certificate.dart';
import '../../../widgets/fingerprint/fingerprint_quiz.dart';
import '../../../widgets/no_shortcuts.dart';

/// The outcome of the quiz and the fingerprint it was asked about.
typedef FingerprintQuizResult = ({bool correct, String fingerprint});

class FingerprintQuizDialog extends StatefulWidget {
  const FingerprintQuizDialog({
    super.key,
    required this.$close,
    required this.host,
  });

  final String host;
  final void Function(FingerprintQuizResult?) $close;

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

  // The first fetch often fails on iOS while the local network permission
  // prompt is still open.
  void _retry() {
    setState(() => _fingerprintFuture = fetchServerCertificate(widget.host));
  }

  void _handleAnswer(String fingerprint, bool correct) {
    widget.$close((correct: correct, fingerprint: fingerprint));
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
                requestErrorText(S.of(context), snapshot.error!),
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
                  onAnswer: (correct) =>
                      _handleAnswer(snapshot.data!, correct),
                )
              ],
            );
          },
        ),
        actions: [
          FutureBuilder<String>(
            future: _fingerprintFuture,
            builder: (context, snapshot) => snapshot.hasError
                ? FilledButton(onPressed: _retry, child: Text(s.playOnRetry))
                : const SizedBox.shrink(),
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
