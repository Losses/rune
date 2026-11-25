import 'package:fluent_ui/fluent_ui.dart';

import '../utils/l10n.dart';

MenuFlyoutItem unavailableMenuEntry(BuildContext context) =>
    MenuFlyoutItem(text: Text(S.of(context).unavailable), onPressed: () {});
