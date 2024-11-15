import 'package:fluent_ui/fluent_ui.dart';

import '../generated/l10n.dart';

MenuFlyoutItem unavailableMenuEntry(BuildContext context) =>
    MenuFlyoutItem(text: Text(S.of(context).unavailable), onPressed: () {});
