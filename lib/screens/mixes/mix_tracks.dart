import '../../utils/api/request_failure.dart';
import '../../utils/l10n.dart';
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
  late Future<(List<(String, String)>, Mix)> mixData;

  @override
  void initState() {
    super.initState();

    mixData = _loadMix();
  }

  Future<(List<(String, String)>, Mix)> _loadMix() async {
    final mix = await getMixById(widget.mixId);
    final queries = await fetchMixQueriesByMixId(widget.mixId);
    return (queries, mix);
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<(List<(String, String)>, Mix)>(
      future: mixData,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return Container();
        } else if (snapshot.hasError) {
          final error = snapshot.error;
          return Center(
            child: Text(
              error is RequestFailure
                  ? (error.kind == RequestErrorKind.notFound
                        ? S.of(context).mixNoLongerExists
                        : requestFailureMessage(S.of(context), error))
                  : 'Error: $error',
            ),
          );
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
