import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../../widgets/tile/fancy_cover.dart';
import '../../widgets/playback_controller/playback_placeholder.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';

const size = 400.0;

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

class _SettingsTestPageState extends State<SettingsTestPage> {
  Future<void> onSelectionChanged(Iterable<String> selectedItems) async {
    debugPrint('${selectedItems.map((i) => i)}');
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Column(children: [
      const NavigationBarPlaceholder(),
      Row(
        children: [Device(theme: theme)],
      ),
      const PlaybackPlaceholder()
    ]);
  }
}

class Device extends StatefulWidget {
  const Device({
    super.key,
    required this.theme,
  });

  final FluentThemeData theme;

  @override
  State<Device> createState() => _DeviceState();
}

class _DeviceState extends State<Device> {
  int configIndex = 0;
  int colorHash = 0;
  Random random = Random();

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: () {
        setState(() {
          configIndex = random.nextInt(9);
          colorHash = random.nextInt(100);
        });
      },
      child: FocusableActionDetector(
          child: Stack(
        alignment: Alignment.center,
        children: [
          SvgPicture.asset(
            'assets/device-layer-1.svg',
            width: size,
            colorFilter: ColorFilter.mode(
              widget.theme.accentColor.normal,
              BlendMode.srcIn,
            ),
          ),
          SvgPicture.asset(
            'assets/device-layer-2.svg',
            width: size,
          ),
          FancyCover(
            size: 135,
            ratio: 9/16,
            texts: (
              "Rune Player",
              "Axiom Design",
              "Version 0.0.5-dev",
            ),
            colorHash: colorHash,
            configIndex: configIndex,
          ),
          SvgPicture.asset(
            'assets/device-layer-3.svg',
            width: size,
          ),
        ],
      )),
    );
  }
}
