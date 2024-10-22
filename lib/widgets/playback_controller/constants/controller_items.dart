import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/api/play_play.dart';
import '../../../utils/api/play_mode.dart';
import '../../../utils/api/play_next.dart';
import '../../../utils/api/play_pause.dart';
import '../../../utils/api/play_previous.dart';
import '../../../utils/dialogs/play_queue_dialog.dart';
import '../../../utils/settings_manager.dart';
import '../../../widgets/playback_controller/fullscreen_button.dart';
import '../../../widgets/playback_controller/utils/playback_mode.dart';
import '../../../providers/status.dart';
import '../../../providers/volume.dart';
import '../../../providers/full_screen.dart';

import '../next_button.dart';
import '../volume_button.dart';
import '../queue_button.dart';
import '../previous_button.dart';
import '../cover_wall_button.dart';
import '../play_pause_button.dart';
import '../playback_mode_button.dart';

class ControllerEntry {
  final String id;
  final IconData Function(BuildContext context) icon;
  final String title;
  final String subtitle;
  final Widget Function(BuildContext context) controllerButtonBuilder;
  final Future<MenuFlyoutItem> Function(BuildContext context)
      flyoutEntryBuilder;
  final List<SingleActivator>? shortcuts;
  final void Function(BuildContext context)? onShortcut;

  ControllerEntry({
    required this.id,
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.controllerButtonBuilder,
    required this.flyoutEntryBuilder,
    required this.shortcuts,
    required this.onShortcut,
  });
}

final controllerItems = [
  ControllerEntry(
    id: 'previous',
    icon: (context) => Symbols.skip_previous,
    title: "Previous",
    subtitle: "Go to the previous track",
    shortcuts: [
      const SingleActivator(LogicalKeyboardKey.arrowLeft, control: true),
    ],
    onShortcut: (context) {
      final statusProvider = Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      if (notReady) return;

      playPrevious();
    },
    controllerButtonBuilder: (context) {
      final statusProvider = Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      return PreviousButton(disabled: notReady);
    },
    flyoutEntryBuilder: (context) async {
      final statusProvider = Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      return MenuFlyoutItem(
        leading: const Icon(Symbols.skip_previous),
        text: const Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text('Previous'),
            ShortcutText('Ctrl+←'),
          ],
        ),
        onPressed: notReady
            ? null
            : () {
                Flyout.of(context).close();
                playPrevious();
              },
      );
    },
  ),
  ControllerEntry(
    id: 'toggle',
    icon: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);

      if (statusProvider.playbackStatus?.state == "Playing") {
        return Symbols.pause;
      } else {
        return Symbols.play_arrow;
      }
    },
    title: "Play/Pause",
    subtitle: "Toggle between play and pause",
    shortcuts: [
      const SingleActivator(LogicalKeyboardKey.space),
      const SingleActivator(LogicalKeyboardKey.keyP, control: true),
    ],
    onShortcut: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      if (notReady) return;

      if (statusProvider.playbackStatus?.state == "Playing") {
        playPause();
      } else {
        playPlay();
      }
    },
    controllerButtonBuilder: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final status = statusProvider.playbackStatus;
      final notReady = statusProvider.notReady;

      return PlayPauseButton(
        disabled: notReady,
        state: status?.state ?? "Stopped",
      );
    },
    flyoutEntryBuilder: (context) async {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final status = statusProvider.playbackStatus;
      final notReady = statusProvider.notReady;

      return MenuFlyoutItem(
        leading: status?.state == "Playing"
            ? const Icon(Symbols.pause)
            : const Icon(Symbols.play_arrow),
        text: Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
          status?.state == "Playing" ? const Text('Pause') : const Text('Play'),
          const ShortcutText('Ctrl+P'),
        ]),
        onPressed: notReady
            ? null
            : () {
                Flyout.of(context).close();
                status?.state == "Playing" ? playPause() : playPlay();
              },
      );
    },
  ),
  ControllerEntry(
    id: 'next',
    icon: (context) => Symbols.skip_next,
    title: "Next",
    subtitle: "Go to the next track",
    shortcuts: [
      const SingleActivator(LogicalKeyboardKey.arrowRight, control: true),
    ],
    onShortcut: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      if (notReady) return;

      playNext();
    },
    controllerButtonBuilder: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      return NextButton(disabled: notReady);
    },
    flyoutEntryBuilder: (context) async {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      return MenuFlyoutItem(
        leading: const Icon(Symbols.skip_next),
        text: const Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text('Next'),
            ShortcutText('Ctrl+→'),
          ],
        ),
        onPressed: notReady
            ? null
            : () {
                Flyout.of(context).close();
                playPrevious();
              },
      );
    },
  ),
  ControllerEntry(
    id: 'volume',
    icon: (context) {
      final volumeProvider = Provider.of<VolumeProvider>(context);

      return volumeProvider.volume > 0.3
          ? Symbols.volume_up
          : volumeProvider.volume > 0
              ? Symbols.volume_down
              : Symbols.volume_mute;
    },
    title: "Volume",
    subtitle: "Adjust the volume",
    shortcuts: [],
    onShortcut: null,
    controllerButtonBuilder: (context) => const VolumeButton(),
    flyoutEntryBuilder: (context) async {
      final volumeProvider =
          Provider.of<VolumeProvider>(context, listen: false);

      return MenuFlyoutItem(
        leading: Icon(
          volumeProvider.volume > 0.3
              ? Symbols.volume_up
              : volumeProvider.volume > 0
                  ? Symbols.volume_down
                  : Symbols.volume_mute,
        ),
        text: SizedBox(
          width: 100,
          height: 20,
          child: Stack(
            alignment: Alignment.centerLeft,
            clipBehavior: Clip.none,
            children: [
              Positioned(
                top: 0,
                left: -24.0,
                right: -28.0,
                child: Transform.scale(
                  scale: 0.8,
                  child: const VolumeController(
                    width: 120,
                    height: 20,
                    vertical: false,
                  ),
                ),
              )
            ],
          ),
        ),
        onPressed: () {},
      );
    },
  ),
  ControllerEntry(
    id: 'mode',
    icon: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);

      final PlaybackMode currentMode = PlaybackModeExtension.fromValue(
        statusProvider.playbackStatus?.playbackMode ?? 0,
      );

      return modeToIcon(currentMode);
    },
    title: "Playback Mode",
    subtitle: "Switch between sequential, repeat, or shuffle",
    controllerButtonBuilder: (context) => const PlaybackModeButton(),
    shortcuts: [],
    onShortcut: null,
    flyoutEntryBuilder: (context) async {
      Typography typography = FluentTheme.of(context).typography;
      Color accentColor = Color.alphaBlend(
        FluentTheme.of(context).inactiveColor.withAlpha(100),
        FluentTheme.of(context).accentColor,
      );

      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final status = statusProvider.playbackStatus;

      final currentMode =
          PlaybackModeExtension.fromValue(status?.playbackMode ?? 0);

      // Retrieve disabled modes
      List<dynamic>? storedDisabledModes = await SettingsManager()
          .getValue<List<dynamic>>('disabledPlaybackModes');
      List<PlaybackMode> disabledModes = storedDisabledModes != null
          ? storedDisabledModes
              .map((index) => PlaybackMode.values[index])
              .toList()
          : [];

      // Filter available modes
      List<PlaybackMode> availableModes = PlaybackMode.values
          .where((mode) => !disabledModes.contains(mode))
          .toList();

      // Ensure at least sequential mode is available
      if (availableModes.isEmpty) {
        availableModes.add(PlaybackMode.sequential);
      }

      return MenuFlyoutSubItem(
        leading: Icon(
          modeToIcon(currentMode),
        ),
        text: const Text('Mode'),
        items: (_) => availableModes.map(
          (x) {
            final isCurrent = x == currentMode;
            final color = isCurrent ? accentColor : null;
            return MenuFlyoutItem(
              text: Text(
                modeToLabel(x),
                style: typography.body?.apply(color: color),
              ),
              leading: Icon(
                modeToIcon(x),
                color: color,
              ),
              onPressed: () {
                Flyout.of(context).close();
                playMode(x.toValue());
              },
            );
          },
        ).toList(),
      );
    },
  ),
  ControllerEntry(
    id: 'playlist',
    icon: (context) => Symbols.list_alt,
    title: "Playlist",
    subtitle: "View the playback queue",
    shortcuts: [
      const SingleActivator(LogicalKeyboardKey.keyQ, control: true),
    ],
    onShortcut: (context) {
      showPlayQueueDialog(context);
    },
    controllerButtonBuilder: (context) => QueueButton(),
    flyoutEntryBuilder: (context) async {
      return MenuFlyoutItem(
        leading: const Icon(Symbols.list_alt),
        text: const Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text('Playlist'),
            ShortcutText('Ctrl+Q'),
          ],
        ),
        onPressed: () {
          Flyout.of(context).close();
          showPlayQueueDialog(context);
        },
      );
    },
  ),
  ControllerEntry(
    id: 'hidden',
    icon: (context) => Symbols.hide_source,
    title: "Hidden",
    subtitle: "Content below will be hidden in the others list",
    shortcuts: [],
    onShortcut: null,
    controllerButtonBuilder: (context) => Container(),
    flyoutEntryBuilder: (context) async {
      return MenuFlyoutItem(
        leading: const Icon(Symbols.hide),
        text: const Text('Hidden'),
        onPressed: () {},
      );
    },
  ),
  ControllerEntry(
    id: 'cover_wall',
    icon: (context) => Symbols.photo,
    title: "Cover Wall",
    subtitle: "Display cover art for a unique ambience",
    shortcuts: [const SingleActivator(LogicalKeyboardKey.keyN, alt: true)],
    onShortcut: (context) {
      showCoverArtWall(context);
    },
    controllerButtonBuilder: (context) => const CoverWallButton(),
    flyoutEntryBuilder: (context) async {
      return MenuFlyoutItem(
        leading: const Icon(Symbols.photo),
        text: const Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text('Cover Wall'),
            ShortcutText('Alt+N'),
          ],
        ),
        onPressed: () {
          Flyout.of(context).close();
          showCoverArtWall(context);
        },
      );
    },
  ),
  ControllerEntry(
    id: 'fullscreen',
    icon: (context) {
      final fullScreen = Provider.of<FullScreenProvider>(context);

      return fullScreen.isFullScreen
          ? Symbols.fullscreen_exit
          : Symbols.fullscreen;
    },
    title: "Fullscreen",
    subtitle: "Enter or exit fullscreen mode",
    shortcuts: [
      const SingleActivator(LogicalKeyboardKey.f11),
    ],
    onShortcut: (context) {
      final fullScreen =
          Provider.of<FullScreenProvider>(context, listen: false);

      fullScreen.setFullScreen(!fullScreen.isFullScreen);
    },
    controllerButtonBuilder: (context) => const FullScreenButton(),
    flyoutEntryBuilder: (context) async {
      final fullScreen =
          Provider.of<FullScreenProvider>(context, listen: false);

      return MenuFlyoutItem(
        leading: fullScreen.isFullScreen
            ? const Icon(Symbols.fullscreen_exit)
            : const Icon(Symbols.fullscreen),
        text: const Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text('Fullscreen'),
            ShortcutText('F11'),
          ],
        ),
        onPressed: () {
          Flyout.of(context).close();
          fullScreen.setFullScreen(!fullScreen.isFullScreen);
        },
      );
    },
  ),
];

class ShortcutText extends StatelessWidget {
  const ShortcutText(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    return Text(
      text,
      style: theme.typography.caption?.apply(
        color: theme.activeColor.withAlpha(80),
      ),
    );
  }
}
