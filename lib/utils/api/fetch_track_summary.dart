import 'request_failure.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../bindings/bindings.dart';

Future<List<(int, String)>> fetchTrackSummary(BuildContext context) async {
  return (await awaitResponse(
    'SearchMediaFileSummaryRequest',
    SearchMediaFileSummaryResponse.rustSignalStream,
    send: () => SearchMediaFileSummaryRequest(n: 50).sendSignalToRust(),
  )).message.result.map((x) => (x.id, x.name)).toList();
}
