import 'package:fluent_ui/fluent_ui.dart';

import '../../bindings/bindings.dart';
import '../l10n.dart';

Future<List<(int, String)>> fetchTrackSummary(BuildContext context) async {
  final labels = {
    AudioAvailability.remoteOnly: S.of(context).audioRemoteReference,
    AudioAvailability.unavailable: S.of(context).audioUnavailable,
    AudioAvailability.importPending: S.of(context).audioImportPending,
  };
  SearchMediaFileSummaryRequest(n: 50).sendSignalToRust();
  return (await SearchMediaFileSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) {
        final label = labels[x.audioAvailability];
        return (x.id, label == null ? x.name : '${x.name} • $label');
      })
      .toList();
}
