import 'package:fluent_ui/fluent_ui.dart';

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
