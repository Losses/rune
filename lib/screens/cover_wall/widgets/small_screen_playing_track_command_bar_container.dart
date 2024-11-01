import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/fetch_flyout_items.dart';
import '../../../widgets/playback_controller/constants/controller_items.dart';
import '../../../providers/status.dart';
import '../../../providers/volume.dart';
import '../../../providers/playback_controller.dart';

import 'cover_wall_command_bar.dart';

class SmallScreenPlayingTrackCommandBarContainer extends StatefulWidget {
  const SmallScreenPlayingTrackCommandBarContainer({
    super.key,
    required this.shadows,
  });

  final List<Shadow> shadows;

  @override
  SmallScreenPlayingTrackCommandBarContainerState createState() =>
      SmallScreenPlayingTrackCommandBarContainerState();
}

class SmallScreenPlayingTrackCommandBarContainerState
    extends State<SmallScreenPlayingTrackCommandBarContainer> {
  Map<String, MenuFlyoutItem> flyoutItems = {};
  bool initialized = false;
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    _fetchFlyoutItems();
  }

  Future<void> _fetchFlyoutItems() async {
    if (initialized) return;
    initialized = true;

    final Map<String, MenuFlyoutItem> itemMap = await fetchFlyoutItems(context);

    if (!context.mounted) {
      return;
    }

    setState(() {
      flyoutItems = itemMap;
    });
  }

  static (List<ControllerEntry>, List<ControllerEntry>)
      playbackControllerSelector(context, controllerProvider) {
    final entries = controllerProvider.entries;
    final hiddenIndex = entries.indexWhere((entry) => entry.id == 'hidden');
    final List<ControllerEntry> visibleEntries =
        hiddenIndex != -1 ? entries.sublist(0, hiddenIndex) : entries;
    final List<ControllerEntry> hiddenEntries =
        hiddenIndex != -1 ? entries.sublist(hiddenIndex + 1) : [];

    return (visibleEntries, hiddenEntries);
  }

  @override
  Widget build(BuildContext context) {
    Provider.of<PlaybackStatusProvider>(context);
    Provider.of<VolumeProvider>(context);

    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 212),
      child: Selector<PlaybackControllerProvider,
          (List<ControllerEntry>, List<ControllerEntry>)>(
        selector: playbackControllerSelector,
        builder: (context, entries, child) {
          return CoverWallCommandBar(
            entries: entries,
            flyoutItems: flyoutItems,
            shadows: widget.shadows,
          );
        },
      ),
    );
  }
}
