import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../messages/playback.pb.dart';

import '../next_button.dart';
import '../volume_button.dart';
import '../playlist_button.dart';
import '../previous_button.dart';
import '../cover_wall_button.dart';
import '../play_pause_button.dart';
import '../playback_mode_button.dart';

class ControllerEntry {
  final String id;
  final IconData icon;
  final String title;
  final String subtitle;
  final Widget Function(bool notReady, PlaybackStatus? status)
      controllerButtonBuilder;

  ControllerEntry({
    required this.id,
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.controllerButtonBuilder,
  });
}

var controllerItems = [
  ControllerEntry(
    id: 'previous',
    icon: Symbols.skip_previous,
    title: "Previous",
    subtitle: "Go to the previous track",
    controllerButtonBuilder: (notReady, status) =>
        PreviousButton(disabled: notReady),
  ),
  ControllerEntry(
    id: 'toggle',
    icon: Symbols.play_arrow,
    title: "Play/Pause",
    subtitle: "Toggle between play and pause",
    controllerButtonBuilder: (notReady, status) =>
        PlayPauseButton(disabled: notReady, state: status?.state ?? "Stopped"),
  ),
  ControllerEntry(
    id: 'next',
    icon: Symbols.skip_next,
    title: "Next",
    subtitle: "Go to the next track",
    controllerButtonBuilder: (notReady, status) =>
        NextButton(disabled: notReady),
  ),
  ControllerEntry(
    id: 'volume',
    icon: Symbols.volume_up,
    title: "Volume",
    subtitle: "Adjust the volume",
    controllerButtonBuilder: (notReady, status) => const VolumeButton(),
  ),
  ControllerEntry(
    id: 'mode',
    icon: Symbols.east,
    title: "Playback Mode",
    subtitle: "Switch between sequential, repeat, or shuffle",
    controllerButtonBuilder: (notReady, status) => const PlaybackModeButton(),
  ),
  ControllerEntry(
    id: 'playlist',
    icon: Symbols.list_alt,
    title: "Playlist",
    subtitle: "View the playback queue",
    controllerButtonBuilder: (notReady, status) => PlaylistButton(),
  ),
  ControllerEntry(
    id: 'hidden',
    icon: Symbols.hide_source,
    title: "Hidden",
    subtitle: "Content below will be hidden in the others list",
    controllerButtonBuilder: (_, __) => Container(),
  ),
  ControllerEntry(
    id: 'cover_wall',
    icon: Symbols.photo,
    title: "Cover Art Wall",
    subtitle: "Display cover art for a unique ambience",
    controllerButtonBuilder: (notReady, status) => const CoverWallButton(),
  ),
];
