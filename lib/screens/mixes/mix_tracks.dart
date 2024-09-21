import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/screens/query_tracks/query_tracks.dart';

import '../../utils/dialogs/mix/utils.dart';

class MixTrackesPage extends StatefulWidget {
  final int mixId;

  const MixTrackesPage({super.key, required this.mixId});

  @override
  State<MixTrackesPage> createState() => _MixTrackesPageState();
}

class _MixTrackesPageState extends State<MixTrackesPage> {
  late Future<List<(String, String)>> queries;

  @override
  void initState() {
    super.initState();

    queries = fetchMixQueriesByQueryId(widget.mixId);
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<List<(String, String)>>(
      future: queries,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return Container();
        } else if (snapshot.hasError) {
          return Center(child: Text('Error: ${snapshot.error}'));
        } else {
          return QueryTracksPage(queries: snapshot.data!);
        }
      },
    );
  }
}
