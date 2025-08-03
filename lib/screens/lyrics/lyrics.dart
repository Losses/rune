import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/playing_item.dart';
import '../../utils/api/get_lyric_by_track_id.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../bindings/bindings.dart';
import '../../providers/status.dart';
import '../../providers/responsive_providers.dart';

import 'band_screen_lyrics.dart';
import 'widgets/lyrics_layout.dart';

class LyricsPage extends StatefulWidget {
  const LyricsPage({super.key});

  @override
  State<LyricsPage> createState() => _LyricsPageState();
}

class _LyricsPageState extends State<LyricsPage> {
  PlayingItem? _cachedPlayingItem;
  Future<List<LyricContentLine>>? _lyric;
  List<LyricContentLine> _rawLyrics = [];

  late PlaybackStatusProvider playbackStatus;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    playbackStatus =
        Provider.of<PlaybackStatusProvider>(context, listen: false);

    playbackStatus.addListener(_handlePlaybackStatusUpdate);
    _handlePlaybackStatusUpdate();
  }

  @override
  void dispose() {
    super.dispose();
    playbackStatus.removeListener(_handlePlaybackStatusUpdate);
  }

  List<LyricContentLine> _createNormalizedLyrics(
      List<LyricContentLine> originalLyrics, int trackDuration) {
    return originalLyrics.map((line) {
      var newLine = LyricContentLine(
        startTime: line.startTime,
        endTime: line.endTime >= 599940000 ? trackDuration : line.endTime,
        sections: line.sections
            .map((section) => LyricContentLineSection(
                  startTime: section.startTime,
                  endTime: section.endTime >= 599940000
                      ? trackDuration
                      : section.endTime,
                  content: section.content,
                ))
            .toList(),
      );
      return newLine;
    }).toList();
  }

  _handlePlaybackStatusUpdate() {
    if (_cachedPlayingItem != playbackStatus.playingItem) {
      setState(() {
        final item = playbackStatus.playingItem;
        _cachedPlayingItem = item;
        _lyric = getLyricByTrackId(item).then((lyrics) {
          _rawLyrics = lyrics;
          return lyrics;
        });
      });
    }
  }

  (int, int) _selectProgress(
      BuildContext context, PlaybackStatusProvider status) {
    return (
      (status.playbackStatus.progressSeconds * 1000).round(),
      (status.playbackStatus.duration * 1000).round(),
    );
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder(
      future: _lyric,
      builder: (context, snapshot) {
        if (snapshot.data == null) return Container();

        return Selector<PlaybackStatusProvider, (int, int)>(
          selector: _selectProgress,
          builder: (context, x, child) {
            final List<int> activeLines = [];

            final currentTimeMilliseconds = x.$1;
            final trackDuration = x.$2;

            final normalizedLyrics =
                _createNormalizedLyrics(_rawLyrics, trackDuration);

            for (final (index, line) in normalizedLyrics.indexed) {
              if (currentTimeMilliseconds > line.startTime &&
                  currentTimeMilliseconds < line.endTime) {
                activeLines.add(index);
              }
            }

            return DeviceTypeBuilder(
              deviceType: const [
                DeviceType.band,
                DeviceType.dock,
                DeviceType.zune,
                DeviceType.tv
              ],
              builder: (context, activeBreakpoint) {
                if (activeBreakpoint == DeviceType.dock ||
                    activeBreakpoint == DeviceType.band) {
                  return PageContentFrame(
                    child: BandScreenLyricsView(
                      item: _cachedPlayingItem,
                      lyrics: normalizedLyrics,
                      currentTimeMilliseconds: currentTimeMilliseconds,
                      activeLines: activeLines,
                    ),
                  );
                }

                return LyricsLayout(
                  item: _cachedPlayingItem,
                  lyrics: normalizedLyrics,
                  currentTimeMilliseconds: currentTimeMilliseconds,
                  activeLines: activeLines,
                );
              },
            );
          },
        );
      },
    );
  }
}
