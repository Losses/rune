import 'package:fluent_ui/fluent_ui.dart';

class FingerprintFigure extends StatelessWidget {
  const FingerprintFigure({
    super.key,
    required this.fingerprint,
    this.textStyle = const TextStyle(
      fontFamily: 'NotoRunic',
      fontSize: 20,
      letterSpacing: 4,
    ),
    this.buttonStyle,
    this.mask = const <int>{},
  });

  final String? fingerprint;
  final TextStyle textStyle;
  final ButtonStyle? buttonStyle;
  final Set<int> mask;

  @override
  Widget build(BuildContext context) {
    final localFingerprint = fingerprint;

    return LayoutBuilder(
      builder: (context, constraints) {
        return GridView.count(
          crossAxisCount: 4,
          childAspectRatio: 2,
          mainAxisSpacing: 4,
          crossAxisSpacing: 4,
          physics: const NeverScrollableScrollPhysics(),
          shrinkWrap: true,
          children: List.generate(20, (index) {
            final startIndex = index * 2;
            final text = localFingerprint == null ||
                    startIndex >= localFingerprint.length
                ? ''
                : '${localFingerprint[startIndex]}${startIndex + 1 < localFingerprint.length ? localFingerprint[startIndex + 1] : ''}';

            final masked = mask.contains(index);

            if (masked) {
              return FilledButton(
                onPressed: () {},
                style: buttonStyle,
                child: Text(
                  "",
                  style: textStyle,
                  textAlign: TextAlign.center,
                ),
              );
            }

            return Button(
              onPressed: () {},
              style: buttonStyle,
              child: Text(
                text,
                style: textStyle,
                textAlign: TextAlign.center,
              ),
            );
          }),
        );
      },
    );
  }
}
