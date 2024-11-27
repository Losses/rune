import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/l10n.dart';
import '../../utils/router/navigation.dart';
import '../../providers/router_path.dart';


class BackButton extends StatefulWidget {
  const BackButton({
    super.key,
  });

  @override
  State<BackButton> createState() => _BackButtonState();
}

class _BackButtonState extends State<BackButton> {
  @override
  Widget build(BuildContext context) {
    Provider.of<RouterPathProvider>(context);

    return Builder(
      builder: (context) => PaneItem(
        icon: const Center(child: Icon(FluentIcons.back, size: 12.0)),
        title: Text(S.of(context).back),
        body: const SizedBox.shrink(),
        enabled: $canPop(),
      ).build(
        context,
        false,
        () {
          $pop();
          setState(() => {});
        },
        displayMode: PaneDisplayMode.compact,
      ),
    );
  }
}
