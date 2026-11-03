import 'dart:async';

import '../theme.dart';

final appTheme = AppTheme();

void updateTheme() {
  unawaited(appTheme.setEffect());
}
