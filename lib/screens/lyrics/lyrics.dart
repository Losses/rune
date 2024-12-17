import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/api/get_lyric_by_track_id.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../messages/all.dart';
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
  int _cachedTrackId = -1;
  Future<List<LyricContentLine>>? _lyric;

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

  _handlePlaybackStatusUpdate() {
    if (_cachedTrackId != playbackStatus.playbackStatus.id) {
      setState(() {
        final id = playbackStatus.playbackStatus.id;
        _cachedTrackId = id;
        _lyric = getLyricByTrackId(id);
      });
    }
  }

  int _selectProgress(BuildContext context, PlaybackStatusProvider status) {
    return (status.playbackStatus.progressSeconds * 1000).round();
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder(
      future: _lyric,
      builder: (context, snapshot) {
        if (snapshot.data == null) return Container();

        return Selector<PlaybackStatusProvider, int>(
          selector: _selectProgress,
          builder: (context, currentTimeMilliseconds, child) {
            final List<int> activeLines = [];

            for (final (index, line) in snapshot.data!.indexed) {
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
                      id: _cachedTrackId,
                      lyrics: snapshot.data!,
                      currentTimeMilliseconds: currentTimeMilliseconds,
                      activeLines: activeLines,
                    ),
                  );
                }

                return LyricsLayout(
                  id: _cachedTrackId,
                  lyrics: snapshot.data!,
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
