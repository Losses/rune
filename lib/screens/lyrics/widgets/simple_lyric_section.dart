import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/responsive_providers.dart';

const fontWeight = FontWeight.w600;

class SimpleLyricSection extends StatelessWidget {
  final LyricContentLineSection section;
  final bool isPassed;

  const SimpleLyricSection({
    super.key,
    required this.section,
    required this.isPassed,
  });

  @override
  Widget build(BuildContext context) {
    return _LyricText(
      content: section.content,
      isPassed: isPassed,
      alpha: isPassed ? 255 : 160,
    );
  }
}

class _LyricText extends StatelessWidget {
  final String content;
  final bool isPassed;
  final int alpha;

  const _LyricText({
    required this.content,
    required this.isPassed,
    required this.alpha,
  });

  @override
  Widget build(BuildContext context) {
    final r = Provider.of<ResponsiveProvider>(context);
    final isMini = r.smallerOrEqualTo(DeviceType.zune, false);

    return Text(
      content,
      style: TextStyle(
        fontSize: isMini ? 24 : 32,
        fontWeight: fontWeight,
        color: Colors.white.withAlpha(alpha),
      ),
    );
  }
}
