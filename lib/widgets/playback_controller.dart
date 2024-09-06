import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:reorderables/reorderables.dart';
import 'package:provider/provider.dart';

import '../providers/status.dart';
import '../providers/playlist.dart';
import '../messages/playback.pb.dart';

import './fft_visualize.dart';

const playbackControllerHeight = 80.0;

class PlaybackPlaceholder extends StatelessWidget {
  const PlaybackPlaceholder({super.key});

  @override
  Widget build(BuildContext context) {
    return const SizedBox(height: playbackControllerHeight);
  }
}

String formatTime(double seconds) {
  int totalSeconds = seconds.floor();

  int minutes = totalSeconds ~/ 60;
  int remainingSeconds = totalSeconds % 60;

  String minutesStr = minutes.toString().padLeft(2, '0');
  String secondsStr = remainingSeconds.toString().padLeft(2, '0');

  return '$minutesStr:$secondsStr';
}

class PreviousButton extends StatelessWidget {
  final bool disabled;

  const PreviousButton({required this.disabled, super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: disabled
          ? null
          : () {
              PreviousRequest().sendSignalToRust(); // GENERATED
            },
      icon: const Icon(Symbols.skip_previous),
    );
  }
}

class PlayPauseButton extends StatelessWidget {
  final bool disabled;

  final String state;

  const PlayPauseButton(
      {required this.disabled, required this.state, super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: disabled
          ? null
          : () {
              switch (state) {
                case "Paused":
                case "Stopped":
                  PlayRequest().sendSignalToRust(); // GENERATED
                  break;
                case "Playing":
                  PauseRequest().sendSignalToRust(); // GENERATED
                  break;
              }
            },
      icon: state == "Playing"
          ? const Icon(Symbols.pause)
          : const Icon(Symbols.play_arrow),
    );
  }
}

class NextButton extends StatelessWidget {
  final bool disabled;

  const NextButton({required this.disabled, super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: disabled
          ? null
          : () {
              NextRequest().sendSignalToRust(); // GENERATED
            },
      icon: const Icon(Symbols.skip_next),
    );
  }
}

class PlaylistButton extends StatelessWidget {
  PlaylistButton({super.key});

  final contextController = FlyoutController();

  openContextMenu(BuildContext context) {
    contextController.showFlyout(
      barrierColor: Colors.black.withOpacity(0.1),
      autoModeConfiguration: FlyoutAutoConfiguration(
        preferredMode: FlyoutPlacementMode.topCenter,
      ),
      builder: (context) {
        Typography typography = FluentTheme.of(context).typography;
        Color accentColor = Color.alphaBlend(
          FluentTheme.of(context).inactiveColor.withAlpha(100),
          FluentTheme.of(context).accentColor,
        );

        return Selector<PlaybackStatusProvider, (int?, int?)>(
            selector: (context, playbackStatusProvider) => (
                  playbackStatusProvider.playbackStatus?.index,
                  playbackStatusProvider.playbackStatus?.id
                ),
            builder: (context, playbackStatusProvider, child) {
              return Consumer<PlaylistProvider>(
                  builder: (context, playlistProvider, child) {
                List<Widget> items = playlistProvider.items.map((item) {
                  var isCurrent = playbackStatusProvider.$1 == item.index &&
                      playbackStatusProvider.$2 == item.entry.id;
                  var color = isCurrent ? accentColor : null;

                  return ListTile.selectable(
                    key: ValueKey(item.entry.id),
                    title: Transform.translate(
                      offset: const Offset(-8, 0),
                      child: Row(
                        children: [
                          isCurrent
                              ? Icon(Symbols.play_arrow, color: color, size: 24)
                              : const SizedBox(width: 24),
                          const SizedBox(
                            width: 4,
                          ),
                          SizedBox(
                            width: 320,
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(item.entry.title,
                                    overflow: TextOverflow.ellipsis,
                                    style:
                                        typography.body?.apply(color: color)),
                                Opacity(
                                  opacity: isCurrent ? 0.8 : 0.46,
                                  child: Text(item.entry.artist,
                                      overflow: TextOverflow.ellipsis,
                                      style: typography.caption
                                          ?.apply(color: color)),
                                ),
                              ],
                            ),
                          )
                        ],
                      ),
                    ),
                    onPressed: () =>
                        SwitchRequest(index: item.index).sendSignalToRust(),
                  );
                }).toList();

                if (items.isEmpty) {
                  items.add(
                    ListTile.selectable(
                      key: const Key("disabled"),
                      leading: const Icon(Symbols.info),
                      title: const Text('No items in playlist'),
                      onPressed: () {},
                    ),
                  );
                }

                void onReorder(int oldIndex, int newIndex) {
                  playlistProvider.reorderItems(oldIndex, newIndex);
                }

                return LayoutBuilder(builder:
                    (BuildContext context, BoxConstraints constraints) {
                  double maxHeight = constraints.maxHeight - 100;

                  return ConstrainedBox(
                      constraints: BoxConstraints(
                        maxHeight: maxHeight,
                        maxWidth: 400,
                      ),
                      child: FlyoutContent(
                        child: ReorderableColumn(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          onReorder: onReorder,
                          children: items,
                        ),
                      ));
                });
              });
            });
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    return FlyoutTarget(
      controller: contextController,
      child: IconButton(
        onPressed: () {
          openContextMenu(context);
        },
        icon: const Icon(Symbols.list_alt),
      ),
    );
  }
}

class CoverWallButton extends StatelessWidget {
  const CoverWallButton({super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: () {
        final routeState = GoRouterState.of(context);

        if (routeState.fullPath == "/cover_wall") {
          if (context.canPop()) {
            context.pop();
          }
        } else {
          context.push("/cover_wall");
        }
      },
      icon: const Icon(Symbols.photo_frame),
    );
  }
}

class PlaybackController extends StatefulWidget {
  const PlaybackController({super.key});

  @override
  PlaybackControllerState createState() => PlaybackControllerState();
}

class PlaybackControllerState extends State<PlaybackController> {
  @override
  Widget build(BuildContext context) {
    return Consumer<PlaybackStatusProvider>(
      builder: (context, playbackStatusProvider, child) {
        final theme = FluentTheme.of(context);
        final typography = theme.typography;

        final s = playbackStatusProvider.playbackStatus;

        const scaleY = 0.9;

        return SizedBox(
          height: playbackControllerHeight,
          child: Stack(
            fit: StackFit.expand,
            alignment: Alignment.centerRight,
            children: <Widget>[
              SizedBox.expand(
                  child: Center(
                      child: Container(
                          constraints: const BoxConstraints(
                              minWidth: 1200, maxWidth: 1600),
                          child: Transform(
                            transform: Matrix4.identity()
                              ..scale(1.0, scaleY)
                              ..translate(0.0, (1 - scaleY) * 100),
                            child: const FFTVisualize(),
                          )))),
              SizedBox.expand(
                child: Center(
                  child: Container(
                    constraints:
                        const BoxConstraints(minWidth: 200, maxWidth: 400),
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Text(
                          s != null ? s.title : "",
                          overflow: TextOverflow.ellipsis,
                        ),
                        Slider(
                          value: s != null ? s.progressPercentage * 100 : 0,
                          onChanged: s != null
                              ? (v) => SeekRequest(
                                      positionSeconds: (v / 100) * s.duration)
                                  .sendSignalToRust()
                              : null,
                          style: const SliderThemeData(useThumbBall: false),
                        ),
                        Row(
                          mainAxisAlignment: MainAxisAlignment.spaceBetween,
                          children: [
                            Text(formatTime(s != null ? s.progressSeconds : 0),
                                style: typography.caption),
                            Text(
                                '-${formatTime(s != null ? s.duration - s.progressSeconds : 0)}',
                                style: typography.caption),
                          ],
                        ),
                      ],
                    ),
                  ),
                ),
              ),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  PreviousButton(
                    disabled: s == null,
                  ),
                  PlayPauseButton(
                      disabled: s == null, state: s?.state ?? "Stopped"),
                  NextButton(
                    disabled: s == null,
                  ),
                  PlaylistButton(),
                  const CoverWallButton(),
                  const SizedBox(width: 8),
                ],
              )
            ],
          ),
        );
      },
    );
  }
}
