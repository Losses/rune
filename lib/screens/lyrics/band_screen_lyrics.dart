import 'package:fluent_ui/fluent_ui.dart';

import '../../messages/lyric.pb.dart';
import 'widgets/lyric_display.dart';

class BandScreenLyricsView extends StatefulWidget {
  final int id;
  final List<LyricContentLine> lyrics;
  final int currentTimeMilliseconds;
  final List<int> activeLines;

  const BandScreenLyricsView({
    super.key,
    required this.id,
    required this.lyrics,
    required this.currentTimeMilliseconds,
    required this.activeLines,
  });

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<BandScreenLyricsView> {
  @override
  Widget build(BuildContext context) {
    return LyricsDisplay(
      lyrics: widget.lyrics,
      currentTimeMilliseconds: widget.currentTimeMilliseconds,
      activeLines: widget.activeLines,
    );
  }
}
