import 'package:fluent_ui/fluent_ui.dart';

import '../l10n/app_localizations.dart';

class S {
  static AppLocalizations of(BuildContext context) {
    final instance = AppLocalizations.of(context);
    assert(
      instance != null,
      'No instance of AppLocalizations present in the widget tree. Did you add AppLocalizations.delegate in localizationsDelegates?',
    );
    return instance!;
  }
}
