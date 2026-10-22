import 'dart:math';

import 'package:provider/provider.dart';
import 'package:flutter/scheduler.dart';
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

// The end time the lyric parser gives the last line (`DUMMY_END_TIME`, 9999
// minutes); it is replaced with the track duration.
const _sentinelEndTime = 599940000;

// How far the interpolated position may run ahead of the last real update.
const _maxInterpolationLeadMs = 150.0;

// A real position further than this from the extrapolated one is a seek.
const _seekThresholdMs = 300.0;

class _LyricsPageState extends State<LyricsPage>
    with SingleTickerProviderStateMixin {
  PlayingItem? _cachedPlayingItem;
  Future<List<LyricContentLine>>? _lyric;
  List<LyricContentLine> _rawLyrics = [];

  List<LyricContentLine> _normalizedLyrics = [];
  List<LyricContentLine>? _normalizedSource;
  int _normalizedDuration = -1;

  late PlaybackStatusProvider playbackStatus;

  // Playback position in milliseconds, extrapolated between the 100 ms status
  // updates. Only the active line's sections listen to it.
  final ValueNotifier<double> _interpolatedTime = ValueNotifier(0);
  final Stopwatch _clock = Stopwatch()..start();
  late final Ticker _ticker = createTicker(_onTick);
  double _anchorMs = 0;
  int _anchorMicros = 0;
  double _durationMs = 0;
  bool _isPlaying = false;

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
    playbackStatus.removeListener(_handlePlaybackStatusUpdate);
    _ticker.dispose();
    _interpolatedTime.dispose();
    super.dispose();
  }

  void _onTick(Duration _) {
    final elapsedMs = (_clock.elapsedMicroseconds - _anchorMicros) / 1000.0;
    var time = _anchorMs + min(elapsedMs, _maxInterpolationLeadMs);
    if (_durationMs > 0) time = min(time, _durationMs);

    // Never go backwards between real updates.
    if (time > _interpolatedTime.value) {
      _interpolatedTime.value = time;
    }
  }

  void _syncInterpolatedTime() {
    final status = playbackStatus.playbackStatus;
    final position = status.progressSeconds * 1000;
    final playing = status.state == "Playing";
    _durationMs = status.duration * 1000;

    if (position == _anchorMs && playing == _isPlaying) return;

    final nowMicros = _clock.elapsedMicroseconds;
    final elapsedMs = (nowMicros - _anchorMicros) / 1000.0;

    final isSeek = !playing ||
        !_isPlaying ||
        position < _anchorMs ||
        (position - (_anchorMs + elapsedMs)).abs() > _seekThresholdMs;

    if (isSeek) {
      _interpolatedTime.value = position;
    } else if (position > _interpolatedTime.value) {
      _interpolatedTime.value = position;
    }

    _anchorMs = position;
    _anchorMicros = nowMicros;
    _isPlaying = playing;

    if (playing && !_ticker.isActive) {
      _ticker.start();
    } else if (!playing && _ticker.isActive) {
      _ticker.stop();
    }
  }

  // Normalizing allocates every line, so it is redone only when the lyrics or
  // the track duration change. The sentinel end time is only replaced once the
  // duration is known.
  List<LyricContentLine> _getNormalizedLyrics(int trackDuration) {
    if (identical(_normalizedSource, _rawLyrics) &&
        _normalizedDuration == trackDuration) {
      return _normalizedLyrics;
    }

    int normalizeEnd(int endTime) =>
        endTime >= _sentinelEndTime && trackDuration > 0
            ? trackDuration
            : endTime;

    _normalizedSource = _rawLyrics;
    _normalizedDuration = trackDuration;
    _normalizedLyrics = _rawLyrics.map((line) {
      return LyricContentLine(
        startTime: line.startTime,
        endTime: normalizeEnd(line.endTime),
        sections: line.sections
            .map((section) => LyricContentLineSection(
                  startTime: section.startTime,
                  endTime: normalizeEnd(section.endTime),
                  content: section.content,
                ))
            .toList(),
      );
    }).toList();

    return _normalizedLyrics;
  }

  _handlePlaybackStatusUpdate() {
    _syncInterpolatedTime();

    if (_cachedPlayingItem != playbackStatus.playingItem) {
      setState(() {
        final item = playbackStatus.playingItem;
        _cachedPlayingItem = item;
        _rawLyrics = [];
        _lyric = getLyricByTrackId(item).then((lyrics) {
          if (mounted && _cachedPlayingItem == item) {
            _rawLyrics = lyrics;
          }
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
                _getNormalizedLyrics(trackDuration);

            for (final (index, line) in normalizedLyrics.indexed) {
              final isActive = currentTimeMilliseconds >= line.startTime &&
                  (currentTimeMilliseconds < line.endTime ||
                      // A zero-length line is active at its exact start.
                      (line.startTime == line.endTime &&
                          currentTimeMilliseconds == line.startTime));
              if (isActive) activeLines.add(index);
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
                      interpolatedTime: _interpolatedTime,
                    ),
                  );
                }

                return LyricsLayout(
                  item: _cachedPlayingItem,
                  lyrics: normalizedLyrics,
                  currentTimeMilliseconds: currentTimeMilliseconds,
                  activeLines: activeLines,
                  interpolatedTime: _interpolatedTime,
                );
              },
            );
          },
        );
      },
    );
  }
}
