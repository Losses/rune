import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/playback_controller.dart';
import '../../providers/library_path.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/status.dart';
import '../../utils/playing_item.dart';
import '../../widgets/playback_controller/like_button.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';
import '../../widgets/playback_controller/now_playing.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';

void main() {
  testWidgets('now playing displays metadata for another library item', (
    tester,
  ) async {
    final status = _StatusProvider(
      PlaybackStatusState(
        title: 'Other Library Track',
        artist: 'Other Library Artist',
        album: 'Other Library Album',
        duration: 125,
      ),
      PlayingItem.libraryItem(libraryId: 'library-uuid', fileUuid: 'file-uuid'),
    );
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<PlaybackStatusProvider>.value(value: status),
          ChangeNotifierProvider<ResponsiveProvider>.value(
            value: _ResponsiveProvider(),
          ),
          ChangeNotifierProvider<PlaybackControllerProvider>.value(
            value: _PlaybackControllerProvider(),
          ),
        ],
        child: FluentApp(home: RevealEffectContext(child: const NowPlaying())),
      ),
    );

    expect(find.text('Other Library Track'), findsOneWidget);
    expect(find.text('Other Library Artist'), findsOneWidget);
  });

  testWidgets('the now-playing like action is hidden without a local id', (
    tester,
  ) async {
    await tester.pumpWidget(
      ChangeNotifierProvider<LibraryPathProvider>(
        create: (_) => LibraryPathProvider(null),
        child: FluentApp(
          home: LikeButton(
            item: PlayingItem.libraryItem(
              libraryId: 'library-uuid',
              fileUuid: 'file-uuid',
            ),
          ),
        ),
      ),
    );

    expect(find.byIcon(Symbols.favorite), findsNothing);
  });
}

class _StatusProvider extends ChangeNotifier implements PlaybackStatusProvider {
  _StatusProvider(this.playbackStatus, this.playingItem);

  @override
  final PlaybackStatusState playbackStatus;
  @override
  final PlayingItem playingItem;
  @override
  bool get notReady => false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _ResponsiveProvider extends ChangeNotifier implements ResponsiveProvider {
  @override
  bool smallerOrEqualTo(DeviceType deviceType, [bool listen = true]) => true;

  @override
  DeviceType getActiveDeviceType(List<DeviceType> deviceTypes) =>
      DeviceType.tablet;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _PlaybackControllerProvider extends ChangeNotifier
    implements PlaybackControllerProvider {
  @override
  List<ControllerEntry> availableEntries(BuildContext context) => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
