import 'package:analysis_server_plugin/plugin.dart';
import 'package:analysis_server_plugin/registry.dart';

import 'src/no_toast.dart';

final plugin = RuneLintsPlugin();

class RuneLintsPlugin extends Plugin {
  @override
  String get name => 'rune_lints';

  @override
  void register(PluginRegistry registry) {
    registry.registerWarningRule(NoToast());
  }
}
