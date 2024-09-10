import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:player/messages/recommend.pbserver.dart';

import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';

class SettingsHomePage extends StatefulWidget {
  const SettingsHomePage({super.key});

  @override
  State<SettingsHomePage> createState() => _SettingsHomePageState();
}

class _SettingsHomePageState extends State<SettingsHomePage> {
  @override
  Widget build(BuildContext context) {
    return Column(children: [
      const NavigationBarPlaceholder(),
      Padding(
        padding: const EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Button(
                child: const Text("Library"),
                onPressed: () => context.push('/settings/library')),
            Button(
                child: const Text("Test"),
                onPressed: () => context.push('/settings/test')),
            const Text("Mix"),
            Wrap(
              children: [
                Button(
                    child: const Text("Mix 1"),
                    onPressed: () =>
                        RecommendAndPlayMixRequest(mixId: 0).sendSignalToRust()),
                Button(
                    child: const Text("Mix 2"),
                    onPressed: () =>
                        RecommendAndPlayMixRequest(mixId: 1).sendSignalToRust()),
                Button(
                    child: const Text("Mix 3"),
                    onPressed: () =>
                        RecommendAndPlayMixRequest(mixId: 2).sendSignalToRust()),
                Button(
                    child: const Text("Mix 4"),
                    onPressed: () =>
                        RecommendAndPlayMixRequest(mixId: 3).sendSignalToRust()),
                Button(
                    child: const Text("Mix 5"),
                    onPressed: () =>
                        RecommendAndPlayMixRequest(mixId: 4).sendSignalToRust()),
                Button(
                    child: const Text("Mix 6"),
                    onPressed: () =>
                        RecommendAndPlayMixRequest(mixId: 5).sendSignalToRust()),
                Button(
                    child: const Text("Mix 7"),
                    onPressed: () =>
                        RecommendAndPlayMixRequest(mixId: 6).sendSignalToRust()),
                Button(
                    child: const Text("Mix 8"),
                    onPressed: () =>
                        RecommendAndPlayMixRequest(mixId: 7).sendSignalToRust()),
                Button(
                    child: const Text("Mix 9"),
                    onPressed: () =>
                        RecommendAndPlayMixRequest(mixId: 8).sendSignalToRust()),
              ],
            )
          ],
        ),
      )
    ]);
  }
}
