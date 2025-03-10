import 'package:flutter/gestures.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/volume.dart';

import '../../utils/router/router_aware_flyout_controller.dart';
import '../rune_clickable.dart';

class VolumeButton extends StatefulWidget {
  const VolumeButton({
    super.key,
    required this.shadows,
  });

  final List<Shadow>? shadows;

  @override
  VolumeButtonState createState() => VolumeButtonState();
}

void onScroll(VolumeProvider volumeProvider, PointerSignalEvent pointerSignal) {
  if (pointerSignal is PointerScrollEvent) {
    final currentVolume = volumeProvider.volume;
    final delta = pointerSignal.scrollDelta.dy * -0.0005;
    final newVolume = (currentVolume + delta).clamp(0.0, 1.0);
    volumeProvider.updateVolume(newVolume);
  }
}

class VolumeButtonState extends State<VolumeButton> {
  final _flyoutController = RouterAwareFlyoutController();

  @override
  Widget build(BuildContext context) {
    final volumeProvider = Provider.of<VolumeProvider>(context);

    return Listener(
      onPointerSignal: (pointerSignal) {
        onScroll(volumeProvider, pointerSignal);
      },
      child: FlyoutTarget(
        controller: _flyoutController.controller,
        child: RuneClickable(
          child: Icon(
            volumeProvider.volume > 0.3
                ? Symbols.volume_up
                : volumeProvider.volume > 0
                    ? Symbols.volume_down
                    : Symbols.volume_mute,
            shadows: widget.shadows,
          ),
          onPressed: () {
            _flyoutController.showFlyout(
              barrierColor: Colors.transparent,
              builder: (context) {
                return const FlyoutContent(
                  child: VolumeController(
                    width: 40,
                    height: 150,
                  ),
                );
              },
            );
          },
        ),
      ),
    );
  }

  @override
  void dispose() {
    super.dispose();
    _flyoutController.dispose();
  }
}

class VolumeController extends StatelessWidget {
  final double width;
  final double height;
  final bool vertical;

  const VolumeController({
    super.key,
    this.width = double.infinity,
    this.height = double.infinity,
    this.vertical = true,
  });

  @override
  Widget build(BuildContext context) {
    final volumeProvider = Provider.of<VolumeProvider>(context);

    return Listener(
      onPointerSignal: (pointerSignal) {
        onScroll(volumeProvider, pointerSignal);
      },
      child: SizedBox(
        width: width,
        height: height,
        child: Slider(
          vertical: vertical,
          value: volumeProvider.volume * 100,
          onChanged: (value) {
            volumeProvider.updateVolume(value / 100);
          },
          label: '${(volumeProvider.volume * 100).toInt()}%',
        ),
      ),
    );
  }
}
