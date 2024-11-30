import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../main.dart';
import '../../utils/l10n.dart';
import '../../utils/router/navigation.dart';
import '../../utils/settings_manager.dart';

import '../no_shortcuts.dart';
import '../responsive_dialog_actions.dart';

import 'utils/pro_clip.dart';

const String remindLaterTimeKey = 'remindLaterTime';

class NonProMark extends StatefulWidget {
  const NonProMark({super.key});

  @override
  State<NonProMark> createState() => _NonProMarkState();
}

class _NonProMarkState extends State<NonProMark> {
  Future<bool> shouldShowWidget() async {
    if (isPro) return false;

    final remindLaterTime =
        await SettingsManager().getValue<int>(remindLaterTimeKey);
    if (remindLaterTime != null) {
      final remindLaterDate =
          DateTime.fromMicrosecondsSinceEpoch(remindLaterTime);
      final difference = DateTime.now().difference(remindLaterDate).inDays;
      print('DIFFERENCE: $difference');
      return difference >= 180;
    }
    return true;
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<bool>(
      future: shouldShowWidget(),
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return Container();
        }

        if (snapshot.hasData && snapshot.data == true) {
          return Listener(
            onPointerUp: (_) {
              $showModal<void>(
                context,
                (context, $close) => NoShortcuts(
                  ContentDialog(
                    title: Text(S.of(context).evaluationMode),
                    content: Column(
                      mainAxisSize: MainAxisSize.min,
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Text(
                          S.of(context).evaluationModeContent1,
                          style: TextStyle(height: 1.4),
                        ),
                        SizedBox(height: 4),
                        Text(
                          S.of(context).evaluationModeContent2,
                          style: TextStyle(height: 1.4),
                        ),
                        SizedBox(height: 4),
                        Text(
                          S.of(context).evaluationModeContent3,
                          style: TextStyle(height: 1.4),
                        )
                      ],
                    ),
                    actions: [
                      ResponsiveDialogActions(
                        Button(
                          onPressed: () async {
                            await SettingsManager().setValue(remindLaterTimeKey,
                                DateTime.now().microsecondsSinceEpoch);

                            if (context.mounted) {
                              setState(() {});
                            }

                            $close(null);
                          },
                          child: Text(S.of(context).remindMeLater),
                        ),
                        Button(
                          onPressed: () {
                            $close(null);
                          },
                          child: Text(S.of(context).close),
                        ),
                      ),
                    ],
                  ),
                ),
                barrierDismissible: false,
                dismissWithEsc: true,
              );
            },
            child: Stack(
              alignment: Alignment.bottomLeft,
              children: [
                ClipPath(
                  clipper: ProClip(),
                  child: Container(
                    width: 40,
                    height: 40,
                    color: Color.fromARGB(0xff, 0xff, 0x98, 0x00),
                  ),
                ),
                Padding(
                  padding: EdgeInsets.only(left: 3, bottom: 3),
                  child: Icon(
                    Symbols.lightbulb,
                    color: Colors.black,
                    size: 18,
                  ),
                )
              ],
            ),
          );
        }

        return SizedBox.shrink(); // Return an empty widget if not showing
      },
    );
  }
}
