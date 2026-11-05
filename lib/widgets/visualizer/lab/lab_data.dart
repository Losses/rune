import 'dart:typed_data';

/// How many buckets a second of the song has in the lab's series.
const labBucketRate = 20;

/// How loud a section of the song is.
enum LabTier { quiet, normal, high }

/// A part of the song, from the chart.
class LabSection {
  const LabSection({
    required this.start,
    required this.end,
    required this.label,
    required this.tier,
  });

  final double start;
  final double end;
  final String label;
  final LabTier tier;
}

enum LabMarkKind { cut, look, moment, manual, jump }

/// Something that is planned or that happened at a time of the song. Which
/// fields tell something depends on [kind]; the named constructors set them.
class LabMark {
  const LabMark._({
    required this.kind,
    required this.time,
    this.shotName = '',
    this.size = '',
    this.fill = 0,
    this.strong = false,
    this.dress,
    this.first = false,
    this.lookName = '',
    this.effect,
    this.label = '',
    this.from = 0,
  });

  /// A cut to a shot. [size] is wide, top, medium, close or detail. [fill] is
  /// the place of the cut in a burst (1, 2, ...), or 0 for a cut that is not
  /// part of one.
  const LabMark.cut({
    required double time,
    required String shotName,
    required String size,
    int fill = 0,
    bool strong = false,
    String? dress,
    bool first = false,
  }) : this._(
         kind: LabMarkKind.cut,
         time: time,
         shotName: shotName,
         size: size,
         fill: fill,
         strong: strong,
         dress: dress,
         first: first,
       );

  /// A change to a look.
  const LabMark.look({required double time, required String lookName})
    : this._(kind: LabMarkKind.look, time: time, lookName: lookName);

  /// A moment of the music; [effect] is the lens effect on it, or null.
  const LabMark.moment({
    required double time,
    required bool strong,
    String? effect,
  }) : this._(
         kind: LabMarkKind.moment,
         time: time,
         strong: strong,
         effect: effect,
       );

  /// Something done by hand, such as a button pressed.
  const LabMark.manual({required double time, required String label})
    : this._(kind: LabMarkKind.manual, time: time, label: label);

  /// A seek: the song went from [from] to [time].
  const LabMark.jump({required double time, required double from})
    : this._(kind: LabMarkKind.jump, time: time, from: from);

  final LabMarkKind kind;
  final double time;

  // Cuts.
  final String shotName;
  final String size;
  final int fill;
  final bool strong;
  final String? dress;
  final bool first;

  // Looks.
  final String lookName;

  // Moments (with [strong]).
  final String? effect;

  // Manual marks.
  final String label;

  // Jumps.
  final double from;
}

/// How a column's values are drawn and read.
enum LabColumnKind {
  /// The sections of the song, one row.
  sections,

  /// The marks of the plan and of what happened, one row.
  marks,

  /// A number between [LabColumn.lo] and [LabColumn.hi].
  number,

  /// A count of events in a bucket.
  event,

  /// On or off.
  bool,

  /// An index into [LabColumn.categories].
  category,

  /// A fraction of a cycle, 0 to 1.
  phase,
}

/// Whether the plan has a series for a column.
enum LabPlanAvailability {
  /// It has one.
  yes,

  /// It cannot have one, for the value comes from the live audio.
  no,
}

/// A row the timeline can show.
class LabColumn {
  const LabColumn({
    required this.id,
    required this.name,
    required this.group,
    required this.kind,
    this.lo = 0,
    this.hi = 1,
    this.unit,
    this.categories = const [],
    this.plan = LabPlanAvailability.yes,
    this.hasActual = true,
    this.binds = const [],
    this.rest,
  });

  final String id;
  final String name;

  /// The heading the column is listed under.
  final String group;
  final LabColumnKind kind;

  /// The range a number maps to the color ramp from.
  final double lo;
  final double hi;

  /// `bpm` and `s` are shown after a number.
  final String? unit;

  /// The names of a category column's values.
  final List<String> categories;

  final LabPlanAvailability plan;

  /// False for a column with no actual strip, such as the sections.
  final bool hasActual;

  /// For a scene input, the ids of the columns its bindings read.
  final List<String> binds;

  /// For a scene input, the value it sits at when nothing drives it.
  final double? rest;
}

/// What the lab's timeline draws, which a poll replaces as a whole. The plan's
/// series are owned by the data: nobody changes them after they are given. The
/// actual series are the lab's live buffers, which it keeps writing into as
/// the song plays: [actualRevision] changes with what they hold, so whoever
/// draws them repaints when it does.
class LabTimelineData {
  LabTimelineData({
    required this.length,
    this.sections = const [],
    this.planMarks = const [],
    this.actualMarks = const [],
    this.columns = const [],
    this.planSeries = const {},
    this.actualSeries = const {},
    this.actualRevision = 0,
    this.emptyColumns = const {},
    this.songTime = 0,
    this.playing = false,
    this.song = 0,
  });

  /// No song: the timeline shows its frame only.
  static final empty = LabTimelineData(length: 0);

  /// The song's length in seconds.
  final double length;
  final List<LabSection> sections;

  /// Sorted by time.
  final List<LabMark> planMarks;

  /// Sorted by time.
  final List<LabMark> actualMarks;

  final List<LabColumn> columns;

  /// By column id: a value for each bucket (see [labBucketRate]); NaN where
  /// nothing was recorded. A series may end before the song does: past its end
  /// there is no value.
  final Map<String, Float32List> planSeries;
  final Map<String, Float32List> actualSeries;

  /// Changes whenever a value in [actualSeries] does.
  final int actualRevision;

  /// The ids of the columns with nothing to show in this song; the rows can
  /// leave them out.
  final Set<String> emptyColumns;

  /// Where the song is, in seconds, as last read: about ten times a second.
  final double songTime;

  /// Whether the song plays, so that the time runs on between two readings.
  final bool playing;

  /// Which song this is: it changes with the track, and the timeline then
  /// shows the new song whole.
  final int song;

  int get bucketCount => (length * labBucketRate).ceil();

  LabSection? sectionAt(double time) {
    for (final s in sections) {
      if (time < s.end) return s;
    }
    return sections.isEmpty ? null : sections.last;
  }

  LabTimelineData copyWith({
    double? songTime,
    bool? playing,
    List<LabMark>? actualMarks,
    Map<String, Float32List>? actualSeries,
    int? actualRevision,
    Set<String>? emptyColumns,
  }) => LabTimelineData(
    length: length,
    sections: sections,
    planMarks: planMarks,
    actualMarks: actualMarks ?? this.actualMarks,
    columns: columns,
    planSeries: planSeries,
    actualSeries: actualSeries ?? this.actualSeries,
    actualRevision: actualRevision ?? this.actualRevision,
    emptyColumns: emptyColumns ?? this.emptyColumns,
    songTime: songTime ?? this.songTime,
    playing: playing ?? this.playing,
    song: song,
  );
}
