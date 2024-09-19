import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../screens/settings_test/utils/mix_editor_data.dart';
import '../../screens/settings_test/widgets/mix_editor_controller.dart';

import '../../messages/media_file.pb.dart';

import './widgets/mix_editor.dart';

class SettingsMixPage extends StatefulWidget {
  const SettingsMixPage({super.key});

  @override
  State<SettingsMixPage> createState() => _SettingsMixPageState();
}

class _SettingsMixPageState extends State<SettingsMixPage> {
  late final _controller = MixEditorController();

  @override
  void initState() {
    super.initState();
    _controller.addListener(() {
      print(mixEditorDataToQuery(_controller.getData()));
    });
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final height = MediaQuery.of(context).size.height;
    const reduce = 210.0;

    return Column(children: [
      const NavigationBarPlaceholder(),
      Padding(
        padding: const EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 400,
              child: SizedBox(
                height: height - reduce,
                child: MixEditor(controller: _controller),
              ),
            ),
            Expanded(child: Container()),
          ],
        ),
      )
    ]);
  }
}

fetchPlaylistsByIds(List<(String, String)> queries) async {
  final request = MixQueryRequest(
      queries: queries
          .map((x) => MixQuery(operator: x.$1, parameter: x.$2))
          .toList());
  request.sendSignalToRust(); // GENERATED

  return (await MixQueryResponse.rustSignalStream.first).message.result;
}
