import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../utils/display_blank_option.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('the variable gives a number of seconds, and nothing else does', () {
    expect(blankAfterAtLaunch(const {}), isNull);
    expect(blankAfterAtLaunch(const {'RUNE_BLANK_AFTER': '60'}), 60);
    expect(blankAfterAtLaunch(const {'RUNE_BLANK_AFTER': '0'}), 0);
    expect(blankAfterAtLaunch(const {'RUNE_BLANK_AFTER': ''}), isNull);
    expect(blankAfterAtLaunch(const {'RUNE_BLANK_AFTER': 'soon'}), isNull);
    expect(blankAfterAtLaunch(const {'RUNE_BLANK_AFTER': '-5'}), isNull);
  });

  test(
    'the seconds go to the embedder over the display power channel',
    () async {
      final calls = <MethodCall>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('rune/display_power'), (
            call,
          ) async {
            calls.add(call);
            return null;
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(
              const MethodChannel('rune/display_power'),
              null,
            ),
      );

      expect(await DisplayBlanker.setBlankAfter(60), isTrue);

      expect(calls.single.method, 'setBlankAfter');
      expect(calls.single.arguments, 60);
    },
  );

  test('an embedder that refuses the value leaves the display on', () async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('rune/display_power'),
          (call) async => throw PlatformException(code: 'illegal-argument'),
        );
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            const MethodChannel('rune/display_power'),
            null,
          ),
    );

    expect(await DisplayBlanker.setBlankAfter(9999999999), isFalse);
  });

  test('an embedder without the channel leaves the display on', () async {
    expect(await DisplayBlanker.setBlankAfter(60), isFalse);
  });
}
