import 'package:fluent_ui/fluent_ui.dart';

import '../utils/api/request_failure.dart';
import '../utils/l10n.dart';

/// The ring a list that loads page by page shows while its first page loads.
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

/// How many screens beyond its visible part a list that loads while it scrolls
/// builds, and so asks for, ahead of time.
const double lazyLoadScreensAhead = 1.5;

/// The cache extent of a list that loads while it scrolls along [direction]:
/// far enough that what comes next is asked for before its place scrolls into
/// view, as nothing is drawn where it loads.
double lazyLoadCacheExtent(BuildContext context, Axis direction) {
  final size = MediaQuery.sizeOf(context);
  return lazyLoadScreensAhead *
      (direction == Axis.horizontal ? size.width : size.height);
}
