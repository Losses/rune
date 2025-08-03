import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/api/get_mix_by_id.dart';
import '../../utils/api/fetch_mix_queries_by_mix_id.dart';
import '../../screens/query_tracks/query_tracks.dart';
import '../../bindings/bindings.dart';

class MixTrackesPage extends StatefulWidget {
  final int mixId;
  final String? title;

  const MixTrackesPage({super.key, required this.mixId, required this.title});

  @override
  State<MixTrackesPage> createState() => _MixTrackesPageState();
}

class _MixTrackesPageState extends State<MixTrackesPage> {
  late Future<List<(String, String)>> queries;
  late Future<Mix> mix;

  @override
  void initState() {
    super.initState();

    queries = fetchMixQueriesByMixId(widget.mixId);
    mix = getMixById(widget.mixId);
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<(List<(String, String)>, Mix)>(
      future: (queries, mix).wait,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return Container();
        } else if (snapshot.hasError) {
          return Center(child: Text('Error: ${snapshot.error}'));
        } else {
          return QueryTracksPage(
            queries: QueryList(snapshot.data!.$1),
            title: widget.title,
            mode: snapshot.data!.$2.mode,
          );
        }
      },
    );
  }
}
