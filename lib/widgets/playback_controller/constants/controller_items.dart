import 'dart:io' show Platform;

import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
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

import '../lyric_button.dart';
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
  final String Function(BuildContext context) titleBuilder;
  final String Function(BuildContext context) subtitleBuilder;
  final String Function(BuildContext context) tooltipBuilder;
  final Widget Function(BuildContext context, List<Shadow>? shadows)
      controllerButtonBuilder;
  final Future<MenuFlyoutItem> Function(BuildContext context)
      flyoutEntryBuilder;
  final List<SingleActivator>? shortcuts;
  final void Function(BuildContext context)? onShortcut;

  ControllerEntry({
    required this.id,
    required this.icon,
    required this.titleBuilder,
    required this.subtitleBuilder,
    required this.tooltipBuilder,
    required this.controllerButtonBuilder,
    required this.flyoutEntryBuilder,
    required this.shortcuts,
    required this.onShortcut,
  });
}

List<ControllerEntry> controllerItems = [
  ControllerEntry(
    id: 'previous',
    icon: (context) => Symbols.skip_previous,
    titleBuilder: (context) => S.of(context).previous,
    subtitleBuilder: (context) => S.of(context).previousSubtitle,
    shortcuts: [
      const SingleActivator(LogicalKeyboardKey.arrowLeft, control: true),
    ],
    onShortcut: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      if (notReady) return;

      playPrevious();
    },
    tooltipBuilder: (context) => S.of(context).previous,
    controllerButtonBuilder: (context, shadows) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      return PreviousButton(
        disabled: notReady,
        shadows: shadows,
      );
    },
    flyoutEntryBuilder: (context) async {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      return MenuFlyoutItem(
        leading: const Icon(Symbols.skip_previous),
        text: Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(S.of(context).previous),
            const ShortcutText('Ctrl+←'),
          ],
        ),
        onPressed: notReady
            ? null
            : () {
                Navigator.pop(context);
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

      if (statusProvider.playbackStatus.state == "Playing") {
        return Symbols.pause;
      } else {
        return Symbols.play_arrow;
      }
    },
    titleBuilder: (context) => S.of(context).playPause,
    subtitleBuilder: (context) => S.of(context).playPauseSubtitle,
    shortcuts: [
      const SingleActivator(LogicalKeyboardKey.space),
      const SingleActivator(LogicalKeyboardKey.keyP, control: true),
    ],
    onShortcut: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      if (notReady) return;

      if (statusProvider.playbackStatus.state == "Playing") {
        playPause();
      } else {
        playPlay();
      }
    },
    tooltipBuilder: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final status = statusProvider.playbackStatus;

      return status.state == "Playing"
          ? S.of(context).pause
          : S.of(context).play;
    },
    controllerButtonBuilder: (context, shadows) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final status = statusProvider.playbackStatus;
      final notReady = statusProvider.notReady;

      return PlayPauseButton(
        disabled: notReady,
        state: status.state,
        shadows: shadows,
      );
    },
    flyoutEntryBuilder: (context) async {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final status = statusProvider.playbackStatus;
      final notReady = statusProvider.notReady;

      return MenuFlyoutItem(
        leading: status.state == "Playing"
            ? const Icon(Symbols.pause)
            : const Icon(Symbols.play_arrow),
        text: Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
          status.state == "Playing"
              ? Text(S.of(context).pause)
              : Text(S.of(context).play),
          const ShortcutText('Ctrl+P'),
        ]),
        onPressed: notReady
            ? null
            : () {
                Navigator.pop(context);
                status.state == "Playing" ? playPause() : playPlay();
              },
      );
    },
  ),
  ControllerEntry(
    id: 'next',
    icon: (context) => Symbols.skip_next,
    titleBuilder: (context) => S.of(context).next,
    subtitleBuilder: (context) => S.of(context).nextSubtitle,
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
    tooltipBuilder: (context) => S.of(context).next,
    controllerButtonBuilder: (context, shadows) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      return NextButton(
        disabled: notReady,
        shadows: shadows,
      );
    },
    flyoutEntryBuilder: (context) async {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final notReady = statusProvider.notReady;

      return MenuFlyoutItem(
        leading: const Icon(Symbols.skip_next),
        text: Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(S.of(context).next),
            const ShortcutText('Ctrl+→'),
          ],
        ),
        onPressed: notReady
            ? null
            : () {
                Navigator.pop(context);
                playNext();
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
    titleBuilder: (context) => S.of(context).volume,
    subtitleBuilder: (context) => S.of(context).volumeSubtitle,
    shortcuts: [],
    onShortcut: null,
    tooltipBuilder: (context) => S.of(context).volume,
    controllerButtonBuilder: (context, shadows) => VolumeButton(
      shadows: shadows,
    ),
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
        statusProvider.playbackStatus.playbackMode ?? 0,
      );

      return modeToIcon(currentMode);
    },
    titleBuilder: (context) => S.of(context).playbackMode,
    subtitleBuilder: (context) => S.of(context).playbackModeSubtitle,
    tooltipBuilder: (context) {
      final statusProvider =
          Provider.of<PlaybackStatusProvider>(context, listen: false);
      final status = statusProvider.playbackStatus;

      final currentMode =
          PlaybackModeExtension.fromValue(status.playbackMode ?? 0);

      return modeToLabel(context, currentMode);
    },
    controllerButtonBuilder: (context, shadows) => PlaybackModeButton(
      shadows: shadows,
    ),
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
          PlaybackModeExtension.fromValue(status.playbackMode ?? 0);

      // Retrieve disabled modes
      List<dynamic>? storedDisabledModes = await SettingsManager()
          .getValue<List<dynamic>>('disabled_playback_modes');
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
        text: context.mounted ? Text(S.of(context).mode) : Text(""),
        items: (_) => availableModes.map(
          (x) {
            final isCurrent = x == currentMode;
            final color = isCurrent ? accentColor : null;
            return MenuFlyoutItem(
              text: Text(
                modeToLabel(context, x),
                style: typography.body?.apply(color: color),
              ),
              leading: Icon(
                modeToIcon(x),
                color: color,
              ),
              onPressed: () {
                Navigator.pop(context);
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
    titleBuilder: (context) => S.of(context).playlist,
    subtitleBuilder: (context) => S.of(context).playlistSubtitle,
    shortcuts: [
      const SingleActivator(LogicalKeyboardKey.keyQ, control: true),
    ],
    onShortcut: (context) {
      showPlayQueueDialog(context);
    },
    tooltipBuilder: (context) => S.of(context).playlist,
    controllerButtonBuilder: (context, shadows) => QueueButton(
      shadows: shadows,
    ),
    flyoutEntryBuilder: (context) async {
      return MenuFlyoutItem(
        leading: const Icon(Symbols.list_alt),
        text: Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(S.of(context).playlist),
            const ShortcutText('Ctrl+Q'),
          ],
        ),
        onPressed: () {
          Navigator.pop(context);
          showPlayQueueDialog(context);
        },
      );
    },
  ),
  ControllerEntry(
    id: 'hidden',
    icon: (context) => Symbols.hide_source,
    titleBuilder: (context) => S.of(context).hidden,
    subtitleBuilder: (context) => S.of(context).hiddenSubtitle,
    shortcuts: [],
    onShortcut: null,
    tooltipBuilder: (context) => S.of(context).hidden,
    controllerButtonBuilder: (context, shadows) => Container(),
    flyoutEntryBuilder: (context) async {
      return MenuFlyoutItem(
        leading: const Icon(Symbols.hide),
        text: Text(S.of(context).hidden),
        onPressed: () {},
      );
    },
  ),
  ControllerEntry(
    id: 'cover_wall',
    icon: (context) => Symbols.photo,
    titleBuilder: (context) => S.of(context).coverWall,
    subtitleBuilder: (context) => S.of(context).coverWallSubtitle,
    shortcuts: [const SingleActivator(LogicalKeyboardKey.keyN, alt: true)],
    onShortcut: (context) {
      showCoverArtWall();
    },
    tooltipBuilder: (context) => S.of(context).coverWall,
    controllerButtonBuilder: (context, shadows) => CoverWallButton(
      shadows: shadows,
    ),
    flyoutEntryBuilder: (context) async {
      return MenuFlyoutItem(
        leading: const Icon(Symbols.photo),
        text: Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(S.of(context).coverWall),
            ShortcutText(Platform.isMacOS ? '⌥+N' : 'Alt+N'),
          ],
        ),
        onPressed: () {
          Navigator.pop(context);
          showCoverArtWall();
        },
      );
    },
  ),
  ControllerEntry(
    id: 'lyric',
    icon: (context) => Symbols.lyrics,
    titleBuilder: (context) => S.of(context).lyrics,
    subtitleBuilder: (context) => S.of(context).lyricsSubtitle,
    shortcuts: [const SingleActivator(LogicalKeyboardKey.keyL, alt: true)],
    onShortcut: (context) {
      showLyrics();
    },
    tooltipBuilder: (context) => S.of(context).lyrics,
    controllerButtonBuilder: (context, shadows) => LyricsButton(
      shadows: shadows,
    ),
    flyoutEntryBuilder: (context) async {
      return MenuFlyoutItem(
        leading: const Icon(Symbols.lyrics),
        text: Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(S.of(context).lyrics),
            ShortcutText(Platform.isMacOS ? '⌥+L' : 'Alt+L'),
          ],
        ),
        onPressed: () {
          Navigator.pop(context);
          showLyrics();
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
    titleBuilder: (context) => S.of(context).fullscreen,
    subtitleBuilder: (context) => S.of(context).fullscreenSubtitle,
    shortcuts: [
      const SingleActivator(LogicalKeyboardKey.f11),
    ],
    onShortcut: (context) {
      final fullScreen =
          Provider.of<FullScreenProvider>(context, listen: false);

      fullScreen.setFullScreen(!fullScreen.isFullScreen);
    },
    tooltipBuilder: (context) => S.of(context).fullscreen,
    controllerButtonBuilder: (context, shadows) => FullScreenButton(
      shadows: shadows,
    ),
    flyoutEntryBuilder: (context) async {
      final fullScreen =
          Provider.of<FullScreenProvider>(context, listen: false);

      return MenuFlyoutItem(
        leading: fullScreen.isFullScreen
            ? const Icon(Symbols.fullscreen_exit)
            : const Icon(Symbols.fullscreen),
        text: Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(S.of(context).fullscreen),
            const ShortcutText('F11'),
          ],
        ),
        onPressed: () {
          Navigator.pop(context);
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
        color: theme.resources.textFillColorPrimary.withAlpha(80),
      ),
    );
  }
}
