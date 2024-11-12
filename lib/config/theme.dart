import 'package:fluent_ui/fluent_ui.dart';

import '../theme.dart';

final appTheme = AppTheme();

updateTheme() {
  WidgetsBinding.instance.addPostFrameCallback(
    (_) {
      appTheme.setEffect();
    },
  );
}
