import 'package:player/messages/library_home.pbserver.dart';

Future<LibrarySummaryResponse> fetchLibrarySummary() async {
  final fetchLibrarySummary = FetchLibrarySummaryRequest();
  fetchLibrarySummary.sendSignalToRust(); // GENERATED

  final rustSignal = await LibrarySummaryResponse.rustSignalStream.first;
  final librarySummary = rustSignal.message;

  return librarySummary;
}
