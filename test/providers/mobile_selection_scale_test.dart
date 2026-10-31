import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/material.dart' as material;
import 'package:flutter/cupertino.dart' as cupertino;
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/widgets/app_viewport.dart';

void main() {
  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    for (final percentage in AppScaleProvider.supportedPercentages) {
      for (final customControls in [false, true]) {
        testWidgets(
          '$platform toolbar restoration at $percentage% custom=$customControls',
          (tester) async {
            final scale = AppScaleProvider(
              read: () async => percentage,
              write: (_) async {},
            );
            await scale.initialize();
            tester.view.viewInsets = const FakeViewPadding(bottom: 120);
            addTearDown(tester.view.resetViewInsets);
            MediaQueryData? menuMedia;
            final controller = TextEditingController(text: 'some text');
            final focus = FocusNode();
            final scroll = ScrollController();
            await tester.pumpWidget(
              ChangeNotifierProvider.value(
                value: scale,
                child: FluentApp(
                  builder: (context, child) => AppViewport(child: child!),
                  home: SingleChildScrollView(
                    controller: scroll,
                    child: SizedBox(
                      height: 1000,
                      child: Padding(
                        padding: const EdgeInsets.only(top: 100),
                        child: Align(
                          alignment: Alignment.topRight,
                          child: SizedBox(
                            width: 80,
                            height: 40,
                            child: customControls
                                ? TextBox(
                                    controller: controller,
                                    focusNode: focus,
                                    selectionControls: material
                                        .materialTextSelectionHandleControls,
                                    contextMenuBuilder: (context, state) {
                                      menuMedia = MediaQuery.of(context);
                                      return TextBox.adaptiveContextMenuBuilder(
                                        context,
                                        state,
                                      );
                                    },
                                  )
                                : TextBox(
                                    controller: controller,
                                    focusNode: focus,
                                  ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            );
            await tester.tap(find.byType(TextBox));
            controller.selection = const TextSelection(
              baseOffset: 0,
              extentOffset: 4,
            );
            await tester.pumpAndSettle();
            final editable = tester.state<EditableTextState>(
              find.byType(EditableText),
            );
            expect(editable.showToolbar(), isTrue);
            await tester.pumpAndSettle();
            expect(editable.selectionOverlay!.toolbarIsVisible, isTrue);
            if (!customControls) {
              final fieldRect = tester.getRect(find.byType(TextBox));
              final copyRect = tester.getRect(find.text('Copy').first);
              final toolbar = tester.getRect(
                find
                    .ancestor(
                      of: find.text('Copy').first,
                      matching: find.byType(Acrylic),
                    )
                    .first,
              );
              expect(toolbar.left, greaterThanOrEqualTo(-1));
              expect(toolbar.top, greaterThanOrEqualTo(-1));
              expect(toolbar.right, lessThanOrEqualTo(801));
              expect(toolbar.bottom, lessThanOrEqualTo(601));
              expect(copyRect.right, greaterThan(0));
              expect(copyRect.left, lessThan(800));
              expect(
                (copyRect.center.dy - fieldRect.center.dy).abs(),
                lessThan(150 * scale.scaleFactor),
              );
            }
            if (customControls) {
              final menu = find.byType(
                platform == TargetPlatform.android
                    ? material.TextSelectionToolbar
                    : cupertino.CupertinoTextSelectionToolbar,
              );
              expect(menu, findsOneWidget);
              final rect = tester.getRect(menu);
              final field = tester.getRect(find.byType(TextBox));
              expect(rect.left, greaterThanOrEqualTo(-1));
              expect(rect.top, greaterThanOrEqualTo(-1));
              expect(rect.right, lessThanOrEqualTo(801));
              expect(rect.bottom, lessThanOrEqualTo(601));
              final copy = tester.getRect(find.text('Copy').first);
              expect(copy.left, greaterThanOrEqualTo(-1));
              expect(copy.top, greaterThanOrEqualTo(-1));
              expect(copy.right, lessThanOrEqualTo(801));
              expect(copy.bottom, lessThanOrEqualTo(601));
              final content = find
                  .ancestor(
                    of: find.text('Copy').first,
                    matching: find.byWidgetPredicate(
                      (widget) =>
                          widget.runtimeType.toString() ==
                          (platform == TargetPlatform.android
                              ? '_TextSelectionToolbarContainer'
                              : '_CupertinoTextSelectionToolbarContent'),
                    ),
                  )
                  .first;
              final actualMenu = tester.getRect(content);
              expect(actualMenu.left, greaterThanOrEqualTo(-1));
              expect(actualMenu.top, greaterThanOrEqualTo(-1));
              expect(actualMenu.right, lessThanOrEqualTo(801));
              expect(actualMenu.bottom, lessThanOrEqualTo(601));
              expect(
                actualMenu.inflate(30 * scale.scaleFactor).overlaps(field),
                isTrue,
              );
              expect(
                menuMedia!.size,
                Size(800 / scale.scaleFactor, 600 / scale.scaleFactor),
              );
              expect(
                menuMedia!.viewInsets.bottom,
                120 / tester.view.devicePixelRatio / scale.scaleFactor,
              );
            }
            final transforms = tester.testTextInput.log.where(
              (call) => call.method == 'TextInput.setEditableSizeAndTransform',
            );
            expect(transforms, isNotEmpty);
            final transform =
                (transforms.last.arguments as Map)['transform'] as List;
            expect(transform[0], closeTo(scale.scaleFactor, 0.001));
            final gesture = await tester.startGesture(
              tester.getCenter(find.byType(TextBox)),
            );
            await gesture.moveBy(const Offset(0, -20));
            await tester.pump(const Duration(seconds: 1));
            await gesture.up();
            await tester.pumpAndSettle();
            expect(editable.selectionOverlay!.toolbarIsVisible, isTrue);
            await tester.pumpWidget(const SizedBox());
            controller.dispose();
            focus.dispose();
            scroll.dispose();
            scale.dispose();
          },
          variant: TargetPlatformVariant.only(platform),
        );
      }
    }
  }
}
