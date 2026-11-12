import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../bindings/bindings.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';
import '../utils/visualizer_supported.dart';
import '../utils/api/request_failure.dart';
import '../utils/rune_log.dart';
import '../widgets/visualizer/lab/lab_data.dart';
import '../widgets/visualizer/lab/lab_pip_layout.dart';
import '../widgets/visualizer/lab/lab_plan_data.dart';
import '../widgets/visualizer/lab/lab_rows.dart';
import '../widgets/visualizer/lab/lab_series.dart';
import '../widgets/visualizer/lab/lab_series_adapter.dart';
import '../widgets/visualizer/lab/lab_views.dart';
import 'responsive_providers.dart';
import 'router_path.dart';
import 'status.dart';

/// The name of [transition] as the lab shows it.
String labTransitionName(VisualizerLabTransition transition) =>
    switch (transition) {
      VisualizerLabTransition.flash => 'Flash',
      VisualizerLabTransition.glitch => 'Glitch',
      VisualizerLabTransition.mosaic => 'Mosaic',
      VisualizerLabTransition.fade => 'Fade',
      VisualizerLabTransition.dissolve => 'Dissolve',
    };

/// The label of the mark that [command] leaves when it is played, or null for
/// a command that plays nothing to mark. A cut with a transition is a cut,
/// named after the transition it wears: a Fade Cut.
String? labPlayedLabel(VisualizerLabCommand command) => switch (command) {
  VisualizerLabCommandCut() => 'Cut',
  VisualizerLabCommandCutDressed(:final transition) =>
    '${labTransitionName(transition)} Cut',
  VisualizerLabCommandBurst() => 'Burst',
  VisualizerLabCommandFlare() => 'Flare',
  VisualizerLabCommandLeak() => 'Leak',
  VisualizerLabCommandLensGlitch() => 'Glitch',
  VisualizerLabCommandLensMosaic() => 'Mosaic',
  _ => null,
};

/// The route the lab's overlay shows on.
const kLabPath = '/cover_wall';

/// Whether the app's layout has room for the lab. The smallest ones (zune
/// and dock upright, car and smaller on its side) have no playback controller
/// bar to move to the top, and no room for the lab under it.
bool visualizerLabFits(ResponsiveProvider responsive) =>
    !responsive.smallerOrEqualTo(DeviceType.zune, false) &&
    !responsive.smallerOrEqualTo(DeviceType.car, false);

/// Whether the lab mode is on, so the visualizer page is the lab's overlay:
/// the overlay is open, the cover wall page is on screen, and the layout has
/// room for it. A window that shrinks past that leaves the lab open but hides
/// it, until the window grows again. A widget that calls this rebuilds when it
/// changes.
bool watchVisualizerLabMode(BuildContext context) {
  final open = context.select<VisualizerLabProvider, bool>(
    (p) => p.overlayOpen,
  );
  final onPage = context.select<RouterPathProvider, bool>(
    (p) => p.path == kLabPath,
  );
  final fits = context.select<ResponsiveProvider, bool>(visualizerLabFits);
  return open && onPage && fits;
}

/// How often the lab asks the visualizer what it did. It is fast only while
/// the lab's body is on screen to show it: an overlay that is open but hidden,
/// as in a window too small for the lab, has nothing to draw.
Duration labPollInterval({required bool open, required bool onScreen}) =>
    open && onScreen
    ? VisualizerLabProvider._openInterval
    : VisualizerLabProvider._closedInterval;

/// The lab of the visualizer: while it is on, the cover wall page can show the
/// lab as an overlay over the visualizer. The setting is saved; whether the
/// overlay is open is not.
///
/// While it is on, the provider asks the visualizer what it did by polling,
/// and turns the answers into [timeline] and [now].
class VisualizerLabProvider extends ChangeNotifier {
  static const _openInterval = Duration(milliseconds: 100);
  static const _closedInterval = Duration(milliseconds: 500);
  // The lab asks the visualizer to collect again this often while it says it
  // does not (the native side started again without it, as after a hot
  // restart).
  static const _reEnableEvery = Duration(seconds: 2);
  static const _maxMarks = 3000;

  // The rows, and the views of them.
  late final LabRowsModel _rows = LabRowsModel(
    onSave: (key, value) => $settingsManager.setValue(switch (key) {
      LabRowsModel.rowsKey => kVisualizerLabRowsKey,
      LabRowsModel.viewsKey => kVisualizerLabViewsKey,
      LabRowsModel.displayKey => kVisualizerLabDisplayKey,
      _ => kVisualizerLabViewKey,
    }, value),
  );

  // What the setting says, and whether the machine has a GPU for the scenes.
  // The lab is on only when both hold: without a GPU there is no visualizer
  // to look into, and the setting is out of sight to turn off.
  bool _wanted = false;
  bool? _gpu;
  bool _enabled = false;
  // Counts the tracks the lab followed, for [LabTimelineData.song].
  int _song = 0;
  bool _overlayOpen = false;
  // Whether the lab's body is mounted, which it is while the lab shows.
  bool _bodyShown = false;
  bool _peeking = false;
  double? _labelWidth;
  double? _cardsHeight;
  double? _sceneWidth;
  LabPipLayout _pipLayout = const LabPipLayout();
  bool _disposed = false;

  /// What the timeline draws. Whatever fills the lab replaces the value as
  /// the song plays; the widgets listen to it alone, not to the provider.
  final ValueNotifier<LabTimelineData> timeline = ValueNotifier(
    LabTimelineData.empty,
  );

  /// What the visualizer drew in its last frame, for the cards; none until a
  /// frame was drawn.
  final ValueNotifier<VisualizerLabNow?> now = ValueNotifier(null);

  final LabSeriesStore _series = LabSeriesStore();
  // The length of the chart of the plan on hand, in seconds.
  double _planLength = 0;
  // The plan version the plan was fetched at; none asks again.
  int? _planVersion;
  List<LabSection> _sections = const [];
  List<LabMark> _planMarks = const [];
  double? _songTime;
  final List<LabMark> _observed = [];
  final List<LabMark> _manual = [];
  // The first readout after a change of track may still tell the last moments
  // of the track before, at its times: its events are left out.
  bool _skipEvents = false;

  // What the timeline has been made of, to publish only what changed.
  LabTimelineData _base = LabTimelineData.empty;
  bool _baseDirty = true;
  bool _marksDirty = true;
  List<LabMark> _marksSnapshot = const [];
  double _publishedTime = double.nan;
  bool _publishedPlaying = false;
  Map<String, Float32List>? _publishedActual;
  int _publishedRevision = -1;
  Set<String>? _publishedEmpty;

  PlaybackStatusProvider? _status;
  String? _trackKey;
  Timer? _timer;
  bool _polling = false;
  DateTime? _lastEnableSent;

  VisualizerLabProvider() {
    _rows.addListener(notifyListeners);
    $router.addListener(_onRoute);
    if (isVisualizerSupported) _load();
  }

  // Leaving the cover wall page closes the overlay.
  void _onRoute() {
    if ($router.path != kLabPath) setOverlayOpen(false);
  }

  /// Whether the setting is on.
  bool get enabled => _enabled;

  /// Whether the overlay is asked for. It shows on the cover wall page only.
  bool get overlayOpen => _enabled && _overlayOpen;

  // Whether the lab is open and its body on screen: the polls are fast and
  // the timeline is published only then.
  bool get _watching => overlayOpen && _bodyShown;

  /// Whether the overlay is faded out for a moment to show the visualizer
  /// behind it (not saved). It is false when the overlay is not open.
  bool get peeking => _peeking && overlayOpen;

  /// The rows the timeline shows.
  LabRowSelection get rows => _rows.selection;

  /// The rows and their views, which the timeline and its dialog use.
  LabRowsModel get rowsModel => _rows;

  List<LabColumn> get _columns => timeline.value.columns;

  /// The built-in views and the user's.
  List<LabRowView> get views => _rows.views(_columns);

  /// The current view, or null when it is gone.
  LabRowView? get currentView => _rows.currentView(_columns);

  /// Whether the rows differ from the current view.
  bool get isEdited => _rows.isEdited(_columns);

  void applyView(String name) => _rows.applyView(name, _columns);
  void saveView(String name) => _rows.saveView(name, _columns);
  void renameView(String from, String to) =>
      _rows.renameView(from, to, _columns);
  void updateView(String name) => _rows.updateView(name);
  void deleteView(String name) => _rows.deleteView(name);
  void resetView() => _rows.resetView(_columns);

  /// The width of the timeline's label column; none for the default.
  double? get labelWidth => _labelWidth;

  /// The height of the cards above the timeline; none for as tall as the tallest.
  double? get cardsHeight => _cardsHeight;

  /// The width of the scene's picture; none for as wide as its shape makes it.
  double? get sceneWidth => _sceneWidth;

  /// Where the floating picture of a small window rests, and how wide it is.
  LabPipLayout get pipLayout => _pipLayout;

  Future<void> _load() async {
    final value = await $settingsManager.getValue<bool>(kVisualizerLabKey);
    final rows = await $settingsManager.getValue<String>(kVisualizerLabRowsKey);
    final views = await $settingsManager.getValue<String>(
      kVisualizerLabViewsKey,
    );
    final view = await $settingsManager.getValue<String>(kVisualizerLabViewKey);
    final display = await $settingsManager.getValue<String>(
      kVisualizerLabDisplayKey,
    );
    final labelWidth = await $settingsManager.getValue<num>(
      kVisualizerLabLabelWidthKey,
    );
    final cardsHeight = await $settingsManager.getValue<num>(
      kVisualizerLabCardsHeightKey,
    );
    final sceneWidth = await $settingsManager.getValue<num>(
      kVisualizerLabSceneWidthKey,
    );
    final pipCorner = await $settingsManager.getValue<String>(
      kVisualizerLabPipCornerKey,
    );
    final pipWidth = await $settingsManager.getValue<num>(
      kVisualizerLabPipWidthKey,
    );
    final pipStash = await $settingsManager.getValue<String>(
      kVisualizerLabPipStashKey,
    );
    final pipStashAt = await $settingsManager.getValue<num>(
      kVisualizerLabPipStashAtKey,
    );
    if (_disposed) return;
    _pipLayout = LabPipLayout(
      corner: labPipCornerNames[pipCorner] ?? Alignment.bottomRight,
      width: pipWidth?.toDouble(),
      stash: LabPipSide.values.firstWhere(
        (side) => side.name == pipStash,
        orElse: () => LabPipSide.none,
      ),
      stashAt: (pipStashAt?.toDouble() ?? 0.5).clamp(0.0, 1.0),
    );
    _rows.load(rows: rows, views: views, view: view, display: display);
    _labelWidth = labelWidth?.toDouble();
    _cardsHeight = cardsHeight?.toDouble();
    _sceneWidth = sceneWidth?.toDouble();
    _wanted = value ?? false;
    _applyEnabled(_wanted && _gpu == true);
    notifyListeners();
  }

  /// Told by the lab's body when it is mounted and when it is not. An open
  /// lab that no body shows (the window is too small) is not watched.
  void setBodyShown(bool shown) {
    if (shown == _bodyShown) return;
    final before = _watching;
    _bodyShown = shown;
    if (_enabled && before != _watching) _schedule(immediate: _watching);
  }

  void setPeeking(bool peeking) {
    if (peeking == _peeking) return;
    _peeking = peeking;
    notifyListeners();
  }

  /// Sets the width of the timeline's label column; none for the default.
  /// While a drag goes on, [save] is false: the value follows in memory and is
  /// saved when the drag ends.
  void setLabelWidth(double? width, {bool save = true}) {
    if (width != _labelWidth) {
      _labelWidth = width;
      notifyListeners();
    }
    if (!save) return;
    if (width == null) {
      $settingsManager.removeValue(kVisualizerLabLabelWidthKey);
    } else {
      $settingsManager.setValue(kVisualizerLabLabelWidthKey, width);
    }
  }

  /// Sets the height of the cards above the timeline; none for as tall as the
  /// tallest. While a drag goes on, [save] is false: the value follows in
  /// memory and is saved when the drag ends.
  void setCardsHeight(double? height, {bool save = true}) {
    if (height != _cardsHeight) {
      _cardsHeight = height;
      notifyListeners();
    }
    if (!save) return;
    if (height == null) {
      $settingsManager.removeValue(kVisualizerLabCardsHeightKey);
    } else {
      $settingsManager.setValue(kVisualizerLabCardsHeightKey, height);
    }
  }

  /// Sets the width of the scene's picture; none for as wide as its shape
  /// makes it. While a drag goes on, [save] is false: the value follows in
  /// memory and is saved when the drag ends.
  void setSceneWidth(double? width, {bool save = true}) {
    if (width != _sceneWidth) {
      _sceneWidth = width;
      notifyListeners();
    }
    if (!save) return;
    if (width == null) {
      $settingsManager.removeValue(kVisualizerLabSceneWidthKey);
    } else {
      $settingsManager.setValue(kVisualizerLabSceneWidthKey, width);
    }
  }

  /// Sets where the floating picture rests and how wide it is. It is called
  /// when the picture is let go, so it is saved by default.
  void setPipLayout(LabPipLayout layout, {bool save = true}) {
    if (layout != _pipLayout) {
      _pipLayout = layout;
      notifyListeners();
    }
    if (!save) return;
    $settingsManager.setValue(
      kVisualizerLabPipCornerKey,
      labPipCornerName(layout.corner),
    );
    if (layout.width == null) {
      $settingsManager.removeValue(kVisualizerLabPipWidthKey);
    } else {
      $settingsManager.setValue(kVisualizerLabPipWidthKey, layout.width);
    }
    $settingsManager.setValue(kVisualizerLabPipStashKey, layout.stash.name);
    $settingsManager.setValue(kVisualizerLabPipStashAtKey, layout.stashAt);
  }

  void setRows(LabRowSelection rows) => _rows.setSelection(rows);

  void setEnabled(bool enabled) {
    if (enabled == _wanted) return;
    _wanted = enabled;
    $settingsManager.setValue(kVisualizerLabKey, enabled);
    _applyEnabled(enabled && _gpu == true);
  }

  /// Follows whether the machine has a GPU for the scenes (null until known).
  /// A saved setting that is on takes effect once there is one.
  void attachGpu(bool? gpu) {
    if (gpu == _gpu) return;
    _gpu = gpu;
    // This runs while the tree builds: the change follows right after.
    scheduleMicrotask(() {
      if (!_disposed && _gpu == gpu) _applyEnabled(_wanted && gpu == true);
    });
  }

  void _applyEnabled(bool enabled) {
    if (enabled == _enabled) return;
    _enabled = enabled;
    SetVisualizerLabRequest(enabled: enabled).sendSignalToRust();
    _lastEnableSent = DateTime.now();
    if (enabled) {
      _schedule(immediate: true);
    } else {
      _overlayOpen = false;
      _peeking = false;
      _timer?.cancel();
      _timer = null;
      _clear();
      _publishEmpty();
    }
    notifyListeners();
  }

  /// Follows the track on the player; the lab starts afresh on another one.
  void attachStatus(PlaybackStatusProvider status) {
    _status = status;
    final state = status.playbackStatus;
    final key = state.item ?? state.libPath;
    if (key == _trackKey) return;
    _trackKey = key;
    _song++;
    _clear();
    _skipEvents = true;
    // This runs while the tree builds: the poll shows the empty lab.
    if (_enabled) _schedule(immediate: true);
  }

  // Forgets everything of the song.
  void _clear() {
    _planLength = 0;
    _planVersion = null;
    _sections = const [];
    _planMarks = const [];
    _songTime = null;
    _observed.clear();
    _manual.clear();
    _series.reset();
    _baseDirty = true;
    _marksDirty = true;
  }

  void _publishEmpty() {
    _publishedTime = double.nan;
    _publishedActual = null;
    _publishedRevision = -1;
    _publishedEmpty = null;
    _base = LabTimelineData.empty;
    now.value = null;
    timeline.value = LabTimelineData.empty;
  }

  void setOverlayOpen(bool open) {
    if (open == _overlayOpen) return;
    final before = _watching;
    _overlayOpen = open;
    if (!open) _peeking = false;
    // The poll goes faster while the overlay shows.
    if (_enabled && before != _watching) _schedule(immediate: _watching);
    notifyListeners();
  }

  void toggleOverlay() => setOverlayOpen(!overlayOpen);

  // ---------- polling ----------

  void _schedule({bool immediate = false}) {
    _timer?.cancel();
    _timer = null;
    if (!_enabled || _disposed) return;
    _timer = Timer(
      immediate
          ? Duration.zero
          : labPollInterval(open: overlayOpen, onScreen: _bodyShown),
      _poll,
    );
  }

  Future<void> _poll() async {
    _timer = null;
    if (!_enabled || _disposed || _polling) return;
    _polling = true;
    try {
      final response = (await awaitResponse(
        'VisualizerLabRequest',
        VisualizerLabResponse.rustSignalStream,
        timeout: const Duration(seconds: 1),
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) => VisualizerLabRequest(
          columnsVersion: Uint64(BigInt.from(_series.version)),
          callId: callId,
        ).sendSignalToRust(),
      )).message;
      if (!_enabled || _disposed) return;
      _take(response);
      if (_planVersion != response.planVersion.toInt()) {
        await _fetchPlan();
        if (!_enabled || _disposed) return;
      }
      _publish();
    } on TimeoutException {
      // The next tick asks again.
    } on RequestFailure catch (failure) {
      error$('Could not update the visualizer lab: ${failure.detail}');
    } finally {
      _polling = false;
      if (_enabled && !_disposed && _timer == null) _schedule();
    }
  }

  void _take(VisualizerLabResponse response) {
    if (!response.inspecting) {
      final sent = _lastEnableSent;
      if (sent == null || DateTime.now().difference(sent) > _reEnableEvery) {
        SetVisualizerLabRequest(enabled: true).sendSignalToRust();
        _lastEnableSent = DateTime.now();
      }
    }

    final skipping = _skipEvents;
    final events = skipping ? const <VisualizerLabEvent>[] : response.events;
    _skipEvents = false;

    var jumped = false;
    final lastNow = response.now;
    var before = _songTime;
    for (final event in events) {
      final mark = labMarkOfEvent(event, before);
      if (mark == null) continue;
      if (mark.kind == LabMarkKind.jump) {
        _jumpTo(mark.time);
        jumped = true;
      }
      before = mark.time;
      _observed.add(mark);
      _marksDirty = true;
    }

    // Until a frame is drawn the song time is a placeholder, not a place in
    // the song: it is no seek. The readout that skips its events may tell
    // the track before's time too, so it gives no song time either.
    if (lastNow != null && !skipping) {
      final time = response.songTime;
      final last = _songTime;
      // A seek back that no jump told of (the visualizer was not drawing).
      if (!jumped && last != null && time < last - labSeekBackwards) {
        _jumpTo(time);
        _observed.add(LabMark.jump(time: time, from: last));
        _marksDirty = true;
      }
      _songTime = time;
    }
    now.value = lastNow;

    if (_observed.length > _maxMarks) {
      _observed.removeRange(0, _observed.length - _maxMarks);
      _marksDirty = true;
    }

    final update = labSeriesUpdateFromHub(response);
    final catalogue = update.catalogue;
    if (catalogue != null) {
      final before = _series.version;
      _series.setCatalogue(catalogue);
      if (_series.version != before) {
        // The plan's series followed the catalogue before.
        _planVersion = null;
        _baseDirty = true;
      }
    }
    _syncBuckets();
    _series.addSamples(update.samples);
  }

  // The series are as long as the longer of the chart and the player's
  // duration: the player may run on past the end of the chart.
  void _syncBuckets() {
    final length = _songLength;
    if (length > 0) _series.setBuckets((length * labBucketRate).ceil());
  }

  // How long the song is: the chart's length, or the player's duration where
  // that is longer.
  double get _songLength {
    final duration = _status?.playbackStatus.duration ?? 0;
    final chart = _planLength;
    return math.max(chart, duration);
  }

  /// The visualizer went to song time [time] without playing up to it: what
  /// was seen after it is of another playing.
  void _jumpTo(double time) {
    _observed.removeWhere((m) => m.time > time);
    _manual.removeWhere((m) => m.time > time);
    _marksDirty = true;
  }

  Future<void> _fetchPlan() async {
    try {
      final response = (await awaitResponse(
        'VisualizerPlanRequest',
        VisualizerPlanResponse.rustSignalStream,
        timeout: const Duration(seconds: 1),
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) =>
            VisualizerPlanRequest(callId: callId).sendSignalToRust(),
      )).message;
      if (!_enabled || _disposed) return;
      // None is a plan too: the song has no chart yet, or its plan is being
      // made. The version changes when one comes.
      final plan = response.plan;
      _planLength = plan?.length ?? 0;
      _planVersion = response.version.toInt();
      if (plan == null) {
        _sections = const [];
        _planMarks = const [];
        _series.clearPlan();
      } else {
        _sections = labSectionsOf(plan);
        _planMarks = labPlanMarksOf(plan);
        final series = labSeriesPlanFromHub(plan);
        if (series != null) {
          _series.importPlan(series);
        } else {
          _series.clearPlan();
        }
      }
      _syncBuckets();
      _baseDirty = true;
    } on TimeoutException {
      // The version stays: the next poll asks again.
    } on RequestFailure catch (failure) {
      error$('Could not update the visualizer plan: ${failure.detail}');
    }
  }

  // Gives the timeline what the poll found, when it is something new. While
  // the overlay is closed nothing draws the series or the song time, so the
  // timeline is replaced only when what the slider's markers and the views
  // read changed.
  void _publish() {
    final status = _status?.playbackStatus;
    final length = _songLength;
    final rebuilt = _baseDirty || _base.length != length;
    if (rebuilt) {
      _rows.catalogueChanged(_base.columns, _series.columns);
      _base = LabTimelineData(
        length: length,
        sections: _sections,
        planMarks: _planMarks,
        columns: _series.columns,
        planSeries: _series.planSeries,
        emptyColumns: _series.emptyColumns,
        song: _song,
      );
      _baseDirty = false;
      _publishedActual = null;
      _publishedRevision = -1;
      _publishedEmpty = null;
      _publishedTime = double.nan;
    }
    if (_marksDirty) {
      _marksSnapshot = List.unmodifiable(labMergeMarks(_observed, _manual));
      _marksDirty = false;
      _publishedTime = double.nan;
    }
    if (!_watching && !rebuilt) return;
    final actual = _series.actualSeries;
    final revision = _series.revision;
    final empty = _series.emptyColumns;
    final time = _songTime ?? status?.progressSeconds ?? 0;
    final playing = status?.state == 'Playing';
    if (identical(actual, _publishedActual) &&
        revision == _publishedRevision &&
        identical(empty, _publishedEmpty) &&
        time == _publishedTime &&
        playing == _publishedPlaying) {
      return;
    }
    _publishedActual = actual;
    _publishedRevision = revision;
    _publishedEmpty = empty;
    _publishedTime = time;
    _publishedPlaying = playing;
    timeline.value = _base.copyWith(
      songTime: time,
      playing: playing,
      actualMarks: _marksSnapshot,
      actualSeries: actual,
      actualRevision: revision,
      emptyColumns: empty,
    );
  }

  /// Sends [command]. A command that plays something adds a mark at the song
  /// position, named [markLabel] or after the command.
  void play(VisualizerLabCommand command, {String? markLabel}) {
    VisualizerLabCommandRequest(command: command).sendSignalToRust();
    final label = markLabel ?? labPlayedLabel(command);
    if (label == null) return;
    final time = _songTime ?? _status?.playbackStatus.progressSeconds ?? 0;
    _manual.add(LabMark.manual(time: time, label: label));
    _marksDirty = true;
    if (_enabled) _schedule(immediate: true);
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    now.dispose();
    timeline.dispose();
    _rows.dispose();
    $router.removeListener(_onRoute);
    super.dispose();
  }
}
