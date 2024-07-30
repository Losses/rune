import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:reorderables/reorderables.dart';
import 'package:provider/provider.dart';

import '../providers/status.dart';
import '../providers/playlist.dart';
import '../messages/playback.pb.dart';

import './fft_visualize.dart';

String formatTime(double seconds) {
  int totalSeconds = seconds.floor();

  int minutes = totalSeconds ~/ 60;
  int remainingSeconds = totalSeconds % 60;

  String minutesStr = minutes.toString().padLeft(2, '0');
  String secondsStr = remainingSeconds.toString().padLeft(2, '0');

  return '$minutesStr:$secondsStr';
}

class PreviousButton extends StatelessWidget {
  const PreviousButton({super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: () {
        PreviousRequest().sendSignalToRust(); // GENERATED
      },
      icon: const Icon(Symbols.skip_previous),
    );
  }
}

class PlayPauseButton extends StatelessWidget {
  final String state;

  const PlayPauseButton({required this.state, super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: () {
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
  const NextButton({super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: () {
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
          FluentTheme.of(context).activeColor.withAlpha(100),
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
        final playbackStatus = playbackStatusProvider.playbackStatus;

        if (playbackStatus == null) {
          return const Text("No playback data received yet");
        }

        final state = playbackStatus.state;
        final progressSeconds = playbackStatus.progressSeconds;
        final progressPercentage = playbackStatus.progressPercentage;
        final title = playbackStatus.title;
        final duration = playbackStatus.duration;

        return SizedBox(
          height: 80,
          child: Stack(
            fit: StackFit.expand,
            children: <Widget>[
              const FFTVisualize(),
              Container(
                alignment: Alignment.center,
                child: Row(
                  children: [
                    Expanded(
                      child: Center(
                        child: Container(
                          constraints: const BoxConstraints(
                              minWidth: 200, maxWidth: 400),
                          child: Column(
                            mainAxisAlignment: MainAxisAlignment.center,
                            children: [
                              Text(
                                title,
                                overflow: TextOverflow.ellipsis,
                              ),
                              Slider(
                                value: progressPercentage * 100,
                                onChanged: (v) => SeekRequest(
                                        positionSeconds: (v / 100) * duration)
                                    .sendSignalToRust(),
                                style:
                                    const SliderThemeData(useThumbBall: false),
                              ),
                              Row(
                                mainAxisAlignment:
                                    MainAxisAlignment.spaceBetween,
                                children: [
                                  Text(formatTime(progressSeconds)),
                                  Text(
                                      '-${formatTime(duration - progressSeconds)}'),
                                ],
                              ),
                            ],
                          ),
                        ),
                      ),
                    ),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        const PreviousButton(),
                        PlayPauseButton(state: state),
                        const NextButton(),
                        PlaylistButton(),
                      ],
                    ),
                  ],
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
