import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/rendering.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/visualizer_lab.dart';
import '../../../providers/visualizer_scenes.dart';
import '../../../utils/router/router_aware_flyout_controller.dart';
import 'lab_menu.dart';
import 'lab_section_flow.dart';
import 'lab_segmented.dart';

// The transitions a cut can wear, strong first, as the Camera line lists
// them under Cut.
const _cutTransitions = [
  VisualizerLabTransition.flash,
  VisualizerLabTransition.glitch,
  VisualizerLabTransition.mosaic,
  VisualizerLabTransition.fade,
  VisualizerLabTransition.dissolve,
];

// The Play card's sections are a line each: the name in a column this wide,
// then the controls; a line is as wide as the widest needs, up to the most
// here.
const _nameWidth = 84.0;
const _lineWidth = 460.0;
// The narrowest a column of lines gets, when the card is too narrow for
// them: the name and a button.
const _lineMinWidth = 200.0;

/// The room around the scene's name in its button. The Play card's margin is
/// this much less at its top and left, so that the name stands where the names
/// of the lines below it start while the whole button, which hovers, stays in
/// the card.
const labTitleInset = 8.0;
// The space between two buttons of a line, and between two of its rows.
const _buttonGap = 6.0;

/// Asks for [extra] more width at most than its child does: a [Wrap] leaves
/// the spacing between its children out of its widest intrinsic width, and a
/// line of buttons that fits its content would wrap by that much.
class _WrapSpacing extends SingleChildRenderObjectWidget {
  const _WrapSpacing({required this.extra, required Widget super.child});

  final double extra;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderWrapSpacing(extra);

  @override
  void updateRenderObject(BuildContext context, RenderObject renderObject) {
    (renderObject as _RenderWrapSpacing).extra = extra;
  }
}

class _RenderWrapSpacing extends RenderProxyBox {
  _RenderWrapSpacing(this._extra);

  double _extra;
  set extra(double value) {
    if (value == _extra) return;
    _extra = value;
    markNeedsLayout();
  }

  @override
  double computeMaxIntrinsicWidth(double height) =>
      super.computeMaxIntrinsicWidth(height) + _extra;
}

// The name of a line of the Play card, as the app writes a setting's
// subtitle.
TextStyle? _nameStyle(BuildContext context) {
  final theme = FluentTheme.of(context);
  return theme.typography.caption?.apply(
    color: theme.inactiveColor.withAlpha(160),
  );
}

/// What the Play card shows of [VisualizerLabNow], with what it shows when
/// there is none: the names it lists, what is held and where, the declared
/// effects and the forced level.
class _PlayState {
  const _PlayState({
    required this.hasDirector,
    required this.shotIndex,
    required this.shotHeld,
    required this.shots,
    required this.hasLooks,
    required this.lookNames,
    required this.lookOn,
    required this.lookHeld,
    required this.lens,
    required this.cut,
    required this.forcedLevel,
  });

  factory _PlayState.of(VisualizerLabNow? now) {
    final director = now?.director;
    final looks = now?.looks;
    return _PlayState(
      hasDirector: director != null,
      shotIndex: director?.shotIndex ?? 0,
      shotHeld: director?.held ?? false,
      shots: [
        for (final s in director?.shots ?? const <VisualizerLabShot>[])
          (s.name, s.size.name),
      ],
      hasLooks: looks != null,
      lookNames: looks?.names ?? const [],
      lookOn: looks?.on ?? 0,
      lookHeld: looks?.held ?? false,
      lens: now?.effects.lens ?? const [],
      cut: now?.effects.cut ?? const [],
      forcedLevel: now?.forcedLevel,
    );
  }

  final bool hasDirector;
  final int shotIndex;
  final bool shotHeld;
  final List<(String, String)> shots;
  final bool hasLooks;
  final List<String> lookNames;
  final int lookOn;
  final bool lookHeld;
  final List<String> lens;
  final List<String> cut;
  final VisualizerLabLevel? forcedLevel;
}

/// The scene on screen, by name, as a title that opens a menu of the scenes
/// to show instead: in the order the user keeps them, the one on screen
/// checked, one the user leaves out of the visualizer's turn dimmed, which
/// shows all the same. The cover wall is no scene of the visualizer's.
class LabScenePicker extends StatefulWidget {
  const LabScenePicker({
    super.key,
    required this.scenes,
    required this.current,
    required this.name,
    required this.onPicked,
  });

  final List<VisualizerScene> scenes;

  /// The path of the scene on screen, and its name as the status gives it.
  final String? current;
  final String? name;

  /// Shows the scene at this path.
  final ValueChanged<String> onPicked;

  @override
  State<LabScenePicker> createState() => _LabScenePickerState();
}

class _LabScenePickerState extends State<LabScenePicker> {
  final _controller = RouterAwareFlyoutController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  List<VisualizerScene> get _listed => [
    for (final s in widget.scenes)
      if (!s.isCoverWall) s,
  ];

  void _open() {
    showLabFlyout<void>(
      context,
      _controller,
      placementMode: FlyoutPlacementMode.bottomLeft,
      builder: (context) => MenuFlyout(
        items: [
          for (final s in _listed)
            labChoiceItem(
              label: s.name,
              chosen: s.path == widget.current,
              dim: !s.enabled,
              onPressed: () => widget.onPicked(s.path),
            ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final listed = _listed;
    final title =
        [
          for (final s in listed)
            if (s.path == widget.current) s.name,
        ].firstOrNull ??
        widget.name ??
        'Scene';
    final label = Text(
      title,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      // As the title of a card in the app's settings.
      style: theme.typography.subtitle,
    );
    // With no scene to switch to, the name alone, where the button's would be.
    if (listed.isEmpty) {
      return Padding(
        padding: const EdgeInsets.all(labTitleInset),
        child: label,
      );
    }
    return FlyoutTarget(
      controller: _controller.controller,
      child: IconButton(
        style: const ButtonStyle(
          padding: WidgetStatePropertyAll(EdgeInsets.all(labTitleInset)),
        ),
        onPressed: _open,
        icon: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Flexible(child: label),
            const SizedBox(width: 8),
            const ChevronDown(),
          ],
        ),
      ),
    );
  }
}

/// The body of the Play card: a named line each for the camera and the lens
/// to play, the look and the shot to hold and the level, lines that flow into
/// columns when the card is short, under the [header] when there is one.
class LabPlayBody extends StatelessWidget {
  const LabPlayBody({super.key, required this.lab, this.header});

  final VisualizerLabProvider lab;

  /// What heads the card, such as the [LabScenePicker].
  final Widget? header;

  bool _declared(List<String>? names, String name) =>
      names?.any((n) => n.toLowerCase() == name) ?? false;

  Widget _button(
    String label,
    VisualizerLabCommand command, {
    bool declared = true,
  }) {
    return Opacity(
      opacity: declared ? 1 : 0.45,
      child: Button(onPressed: () => lab.play(command), child: Text(label)),
    );
  }

  /// A section of the card on one line: its [name] in a column of its own,
  /// then its [controls], the name on the baseline of their first line should
  /// they wrap. A list is [centered] on the name instead: fluent's reports the
  /// baseline of its arrow, which stands higher than its text.
  Widget _line(
    BuildContext context,
    String name,
    Widget controls, {
    bool centered = false,
  }) {
    final label = Text(
      name,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: _nameStyle(context),
    );
    return Row(
      crossAxisAlignment: centered
          ? CrossAxisAlignment.center
          : CrossAxisAlignment.baseline,
      textBaseline: TextBaseline.alphabetic,
      children: [
        SizedBox(width: _nameWidth, child: label),
        const SizedBox(width: 8),
        Flexible(child: controls),
      ],
    );
  }

  Widget _buttons(List<Widget> buttons) => _WrapSpacing(
    extra: _buttonGap * (buttons.length - 1),
    child: Wrap(spacing: _buttonGap, runSpacing: _buttonGap, children: buttons),
  );

  /// A list to hold one of [names] from, or none with Auto. It takes the rest
  /// of its line, and a name too long for it ends in an ellipsis on one line,
  /// in the list and in its menu.
  Widget _hold({
    required int? held,
    required List<String> names,
    required ValueChanged<int?> onChanged,
  }) {
    return LabFillWidth(
      child: SizedBox(
        width: double.infinity,
        child: LabChoiceButton<int>(
          isExpanded: true,
          value: held ?? -1,
          items: [
            (-1, 'Auto'),
            for (var i = 0; i < names.length; i++) (i, names[i]),
          ],
          onChanged: (value) => onChanged(value < 0 ? null : value),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    // A readout like the last one does not notify: the lines are built again
    // only when what they show changes.
    // The lines flow into columns down to the card's height; in a card too
    // narrow for the columns they need, they are one column, which scrolls.
    final lines = LayoutBuilder(
      builder: (context, constraints) => SingleChildScrollView(
        primary: false,
        child: ValueListenableBuilder<VisualizerLabNow?>(
          valueListenable: lab.now,
          builder: (context, now, _) =>
              _buildState(context, _PlayState.of(now), constraints.maxHeight),
        ),
      ),
    );
    final header = this.header;
    if (header == null) return lines;
    // The title above the lines, in the room the card's margin leaves it at
    // the top and left (see [labTitleInset]): the lines start where its name
    // does.
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        header,
        const SizedBox(height: 8),
        Flexible(
          child: Padding(
            padding: const EdgeInsets.only(left: labTitleInset),
            child: lines,
          ),
        ),
      ],
    );
  }

  Widget _buildState(
    BuildContext context,
    _PlayState state,
    double flowHeight,
  ) {
    final lens = state.lens;
    final cut = state.cut;
    final hasDirector = state.hasDirector;
    String nameOr(String name, int i) => name.isEmpty ? '#${i + 1}' : name;

    // As wide as the widest row of buttons, which the lists and the choice
    // fill without asking for more.
    return LabSectionFlow(
      columnWidth: _lineWidth,
      fitContent: true,
      minColumnWidth: _lineMinWidth,
      flowHeight: flowHeight,
      sectionGap: 8,
      children: [
        _line(
          context,
          'Camera',
          // A plain cut and a burst, then a cut that wears each transition:
          // one the scene does not list for its cuts is dimmed, and plays
          // all the same.
          Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              _buttons([
                _button(
                  'Cut',
                  const VisualizerLabCommandCut(),
                  declared: hasDirector,
                ),
                _button(
                  'Burst',
                  const VisualizerLabCommandBurst(),
                  declared: hasDirector,
                ),
              ]),
              const SizedBox(height: 6),
              _buttons([
                for (final transition in _cutTransitions)
                  _button(
                    labTransitionName(transition),
                    VisualizerLabCommandCutDressed(transition: transition),
                    declared: _declared(cut, transition.name),
                  ),
              ]),
            ],
          ),
        ),
        _line(
          context,
          'Lens',
          _buttons([
            _button(
              'Flare',
              const VisualizerLabCommandFlare(),
              declared: _declared(lens, 'flare'),
            ),
            _button(
              'Leak',
              const VisualizerLabCommandLeak(),
              declared: _declared(lens, 'leak'),
            ),
            _button(
              'Glitch',
              const VisualizerLabCommandLensGlitch(),
              declared: _declared(lens, 'glitch'),
            ),
            _button(
              'Mosaic',
              const VisualizerLabCommandLensMosaic(),
              declared: _declared(lens, 'mosaic'),
            ),
          ]),
        ),
        if (state.hasLooks)
          _line(
            context,
            'Look',
            centered: true,
            _hold(
              held: state.lookHeld ? state.lookOn : null,
              names: [
                for (var i = 0; i < state.lookNames.length; i++)
                  nameOr(state.lookNames[i], i),
              ],
              onChanged: (index) =>
                  lab.play(VisualizerLabCommandLook(index: index)),
            ),
          ),
        if (hasDirector)
          _line(
            context,
            'Shot',
            centered: true,
            _hold(
              held: state.shotHeld ? state.shotIndex : null,
              names: [
                for (var i = 0; i < state.shots.length; i++)
                  '${nameOr(state.shots[i].$1, i)} (${state.shots[i].$2})',
              ],
              onChanged: (index) => lab.play(
                VisualizerLabCommandShot(index: index),
                markLabel: index == null ? null : 'Shot',
              ),
            ),
          ),
        _line(
          context,
          'Level',
          LabFillWidth(
            child: LabSegmented<VisualizerLabLevel?>(
              value: state.forcedLevel,
              items: const [
                (null, 'Auto'),
                (VisualizerLabLevel.b, 'No Beat'),
                (VisualizerLabLevel.c, 'No Chart'),
              ],
              onChanged: (level) =>
                  lab.play(VisualizerLabCommandForceLevel(level: level)),
            ),
          ),
        ),
      ],
    );
  }
}
