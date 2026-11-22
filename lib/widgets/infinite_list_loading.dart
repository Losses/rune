import 'package:fluent_ui/fluent_ui.dart';

import '../utils/api/request_failure.dart';
import '../utils/l10n.dart';

class InfiniteListLoading extends StatelessWidget {
  const InfiniteListLoading({
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    return const Center(
        child: Padding(
      padding: EdgeInsets.all(8.0),
      child: ProgressRing(),
    ));
  }
}

/// Displays the failed page's error in a tooltip and offers a retry action.
class InfiniteListRetry extends StatelessWidget {
  const InfiniteListRetry({
    super.key,
    required this.error,
    required this.onRetry,
  });

  final Object error;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final strings = S.of(context);
    return Center(
      child: Tooltip(
        message: requestErrorText(strings, error),
        child: Button(onPressed: onRetry, child: Text(strings.playOnRetry)),
      ),
    );
  }
}
