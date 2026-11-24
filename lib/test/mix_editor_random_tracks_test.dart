import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../utils/dialogs/mix/config/mode_select_items.dart';
import '../utils/dialogs/mix/utils/mix_editor_data.dart';
import '../utils/dialogs/mix/widgets/mix_editor_controller.dart';

MixEditorData storedData({int randomTracks = 17}) => MixEditorData(
  title: 'Stored mix',
  group: 'Group',
  randomTracks: randomTracks,
  limit: 0,
  mode: PlaybackMode.shuffle,
  recommendation: '',
  sortBy: 'default',
  sortOrder: true,
  likedOnly: false,
);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('renaming a saved mix preserves its random track count', () {
    final controller = MixEditorController();
    addTearDown(controller.dispose);
    final stored = storedData();

    expect(mixEditorDataToQuery(stored), contains(('lib::random', '17')));
    controller.setData(stored);
    controller.titleController.text = 'Renamed only';

    final outgoing = controller.getData();
    expect(outgoing.title, 'Renamed only');
    expect(outgoing.mode, PlaybackMode.shuffle);
    expect(outgoing.randomTracks, 17);
    expect(mixEditorDataToQuery(outgoing), contains(('lib::random', '17')));
  });

  test('loading a mix without random tracks clears the previous count', () {
    final controller = MixEditorController();
    addTearDown(controller.dispose);

    controller.setData(storedData());
    controller.setData(storedData(randomTracks: 0));

    final outgoing = controller.getData();
    expect(outgoing.randomTracks, 0);
    expect(
      mixEditorDataToQuery(outgoing).where((item) => item.$1 == 'lib::random'),
      isEmpty,
    );
  });

  test('a random track count entered by the user is saved', () {
    final controller = MixEditorController();
    addTearDown(controller.dispose);

    controller.setData(storedData());
    controller.randomTrackController.text = '5';
    controller.modeController.selectedValue = modeSelectValue(
      PlaybackMode.shuffle,
    );

    final outgoing = controller.getData();
    expect(outgoing.randomTracks, 5);
    expect(outgoing.mode, PlaybackMode.shuffle);
    expect(mixEditorDataToQuery(outgoing), contains(('lib::random', '5')));
  });

  test('clearing the random track count saves no random tracks', () {
    final controller = MixEditorController();
    addTearDown(controller.dispose);

    controller.setData(storedData());
    controller.randomTrackController.clear();

    final outgoing = controller.getData();
    expect(outgoing.randomTracks, 0);
    expect(
      mixEditorDataToQuery(outgoing).where((item) => item.$1 == 'lib::random'),
      isEmpty,
    );
  });
}
