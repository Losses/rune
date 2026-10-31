import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';

class _RecordingBinding extends AutomatedTestWidgetsFlutterBinding {
  @override
  ui.SemanticsUpdateBuilder createSemanticsUpdateBuilder() =>
      _RecordingBuilder();
}

class _RecordingBuilder extends Fake implements ui.SemanticsUpdateBuilder {
  static final nodes = <int, Map<Symbol, dynamic>>{};
  static final invalidUpdates = <Set<int>>[];
  final _updated = <int>{};
  final _delegate = ui.SemanticsUpdateBuilder();

  @override
  dynamic noSuchMethod(Invocation invocation) {
    if (invocation.memberName == #updateNode) {
      final id = invocation.namedArguments[#id] as int;
      _updated.add(id);
      nodes[id] = Map.of(invocation.namedArguments);
      return null;
    }
    if (invocation.memberName == #updateCustomAction) return null;
    return super.noSuchMethod(invocation);
  }

  @override
  ui.SemanticsUpdate build() {
    final reachable = <int>{};
    void visit(int id) {
      if (!reachable.add(id)) return;
      final node = nodes[id];
      if (node == null) return;
      for (final child in node[#childrenInTraversalOrder] as List<int>) {
        visit(child);
      }
    }

    if (nodes.containsKey(0)) {
      visit(0);
      final invalid = _updated.difference(reachable);
      if (invalid.isNotEmpty) invalidUpdates.add(invalid);
      nodes.removeWhere((id, _) => !reachable.contains(id));
    }
    return _delegate.build();
  }
}

void main() {
  _RecordingBinding();

  testWidgets(
    'Fluent slider retains accessible actions and reachable overlay semantics',
    (tester) async {
      final handle = tester.ensureSemantics();
      var value = 0.5;
      var scale = 1.0;
      late StateSetter rebuild;
      await tester.pumpWidget(
        FluentApp(
          home: StatefulBuilder(
            builder: (context, setState) {
              rebuild = setState;
              return Center(
                child: Transform.scale(
                  scale: scale,
                  child: Opacity(
                    opacity: 0.9,
                    child: SizedBox(
                      width: 300,
                      child: Slider(
                        value: value,
                        label: 'Volume',
                        onChanged: (next) => setState(() => value = next),
                      ),
                    ),
                  ),
                ),
              );
            },
          ),
        ),
      );
      await tester.drag(find.byType(Slider), const Offset(20, 0));
      await tester.pump();

      for (final nextScale in [0.75, 2.0]) {
        rebuild(() => scale = nextScale);
        await tester.pump();
      }
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      await tester.pump();
      await mouse.moveTo(tester.getCenter(find.byType(Slider)));
      await tester.pump(const Duration(milliseconds: 500));
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.byType(Tooltip), findsOneWidget);
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget is RichText && widget.text.toPlainText() == 'Volume',
        ),
        findsOneWidget,
      );
      final nodes = _RecordingBuilder.nodes;
      expect(_RecordingBuilder.invalidUpdates, isEmpty);
      final sliderNodes = nodes.values.where((node) {
        final actions = node[#actions] as int;
        return actions & ui.SemanticsAction.increase.index != 0 &&
            actions & ui.SemanticsAction.decrease.index != 0;
      });
      expect(sliderNodes, isNotEmpty);
      expect(
        sliderNodes.any((node) => (node[#value] as String).isNotEmpty),
        isTrue,
      );
      final sliderId = sliderNodes.first[#id] as int;
      final before = value;
      tester.binding.renderViews.single.owner!.semanticsOwner!.performAction(
        sliderId,
        ui.SemanticsAction.increase,
      );
      await tester.pump();
      expect(value, greaterThan(before));
      await mouse.removePointer();
      handle.dispose();
    },
  );
}
