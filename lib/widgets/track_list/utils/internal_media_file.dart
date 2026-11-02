import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../../utils/l10n.dart';

class InternalMediaFile {
  final int id;
  final String path;
  final String artist;
  final String album;
  final String title;
  final double duration;
  final String coverArtPath;
  final int? trackNumber;
  final AudioAvailability audioAvailability;
  final String? playlistOccurrenceId;
  final int? playlistPosition;

  InternalMediaFile({
    required this.id,
    required this.path,
    required this.artist,
    required this.album,
    required this.title,
    required this.duration,
    required this.coverArtPath,
    required this.trackNumber,
    this.audioAvailability = AudioAvailability.local,
    this.playlistOccurrenceId,
    this.playlistPosition,
  });

  bool get canPlay =>
      audioAvailability == AudioAvailability.local ||
      audioAvailability == AudioAvailability.remoteOnly;

  String? availabilityLabel(BuildContext context) =>
      switch (audioAvailability) {
        AudioAvailability.local => null,
        AudioAvailability.remoteOnly => S.of(context).audioRemoteReference,
        AudioAvailability.unavailable => S.of(context).audioUnavailable,
        AudioAvailability.importPending => S.of(context).audioImportPending,
      };

  String artistWithAvailability(BuildContext context) {
    final label = availabilityLabel(context);
    return label == null ? artist : '$artist • $label';
  }
}
