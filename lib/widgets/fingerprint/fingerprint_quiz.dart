import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import 'fingerprint_figure.dart';

const base85 = [
  'ᚠ',
  'ᚡ',
  'ᚢ',
  'ᚣ',
  'ᚤ',
  'ᚥ',
  'ᚦ',
  'ᚧ',
  'ᚨ',
  'ᚩ',
  'ᚪ',
  'ᚫ',
  'ᚬ',
  'ᚭ',
  'ᚮ',
  'ᚯ',
  'ᚰ',
  'ᚱ',
  'ᚲ',
  'ᚳ',
  'ᚴ',
  'ᚵ',
  'ᚶ',
  'ᚷ',
  'ᚸ',
  'ᚹ',
  'ᚺ',
  'ᚻ',
  'ᚼ',
  'ᚽ',
  'ᚾ',
  'ᚿ',
  'ᛀ',
  'ᛁ',
  'ᛂ',
  'ᛃ',
  'ᛄ',
  'ᛅ',
  'ᛆ',
  'ᛇ',
  'ᛈ',
  'ᛉ',
  'ᛊ',
  'ᛋ',
  'ᛌ',
  'ᛍ',
  'ᛎ',
  'ᛏ',
  'ᛐ',
  'ᛑ',
  'ᛒ',
  'ᛓ',
  'ᛔ',
  'ᛕ',
  'ᛖ',
  'ᛗ',
  'ᛘ',
  'ᛙ',
  'ᛚ',
  'ᛛ',
  'ᛜ',
  'ᛝ',
  'ᛞ',
  'ᛟ',
  'ᛠ',
  'ᛡ',
  'ᛢ',
  'ᛣ',
  'ᛤ',
  'ᛥ',
  'ᛦ',
  'ᛨ',
  'ᛩ',
  'ᛪ',
  'ᛮ',
  'ᛯ',
  'ᛰ',
  'ᛱ',
  'ᛲ',
  'ᛳ',
  'ᛴ',
  'ᛵ',
  'ᛶ',
  'ᛷ',
  'ᛸ',
];

class FingerprintQuiz extends StatefulWidget {
  const FingerprintQuiz({
    super.key,
    required this.fingerprint,
    required this.onAnswer,
  });

  final String fingerprint;
  final void Function(bool) onAnswer;

  @override
  State<FingerprintQuiz> createState() => _FingerprintQuizState();
}

class _FingerprintQuizState extends State<FingerprintQuiz> {
  late int maskedIndex;
  late String correctAnswer;
  late List<String> options;

  @override
  void initState() {
    super.initState();
    _initializeQuiz();
  }

  void _initializeQuiz() {
    final maxIndex = (widget.fingerprint.length ~/ 2) - 1;
    if (maxIndex < 0) throw FlutterError('Fingerprint too short');

    final random = Random();
    maskedIndex = random.nextInt(maxIndex + 1);

    final start = maskedIndex * 2;
    correctAnswer = widget.fingerprint.substring(start, start + 2);

    options = [correctAnswer, ..._generateWrongAnswers()]..shuffle();
  }

  List<String> _generateWrongAnswers() {
    final random = Random();
    final wrongAnswers = <String>[];

    while (wrongAnswers.length < 3) {
      final a = base85[random.nextInt(base85.length)];
      final b = base85[random.nextInt(base85.length)];
      final candidate = '$a$b';

      if (candidate != correctAnswer && !wrongAnswers.contains(candidate)) {
        wrongAnswers.add(candidate);
      }
    }
    return wrongAnswers;
  }

  void _handleAnswer(int index) {
    final isCorrect = options[index] == correctAnswer;
    widget.onAnswer(isCorrect);
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        FingerprintFigure(
          fingerprint: widget.fingerprint,
          mask: {maskedIndex},
        ),
        const SizedBox(height: 20),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: List.generate(options.length, (index) {
            return Button(
              onPressed: () => _handleAnswer(index),
              child: Text(
                options[index],
                style: const TextStyle(
                  fontFamily: 'NotoRunic',
                  fontSize: 20,
                  letterSpacing: 4,
                ),
              ),
            );
          }),
        ),
      ],
    );
  }
}
