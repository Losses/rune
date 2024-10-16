import 'package:rune/messages/library_home.pbserver.dart';

Future<LibrarySummaryResponse> fetchLibrarySummary() async {
  final fetchLibrarySummary = FetchLibrarySummaryRequest(bakeCoverArts: true);
  fetchLibrarySummary.sendSignalToRust(); // GENERATED

  final rustSignal = await LibrarySummaryResponse.rustSignalStream.first;
  final librarySummary = rustSignal.message;

  return librarySummary;
}
