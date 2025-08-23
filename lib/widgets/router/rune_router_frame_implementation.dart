import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:desktop_drop/desktop_drop.dart';
import 'package:fast_file_picker/fast_file_picker.dart';

import '../../main.dart';

import '../../utils/query_list.dart';
import '../../utils/playing_item.dart';
import '../../utils/filter_audio_files.dart';
import '../../utils/is_cover_art_wall_layout.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../screens/bsod/bsod.dart';
import '../../widgets/navigation_bar/flip_animation.dart';
import '../../widgets/navigation_bar/navigation_bar.dart';
import '../../widgets/navigation_bar/navigation_back_button.dart';
import '../../widgets/banding_animation/branding_animation.dart';
import '../../widgets/playback_controller/cover_art_disk.dart';
import '../../widgets/playback_controller/playback_controller.dart';

import '../../providers/crash.dart';
import '../../providers/router_path.dart';
import '../../providers/library_path.dart';
import '../../providers/responsive_providers.dart';

import '../../bindings/bindings.dart';

import 'rune_stack.dart';
import 'scale_fade_container.dart';

class RuneRouterFrameImplementation extends StatefulWidget {
  const RuneRouterFrameImplementation({super.key, required this.child});

  final Widget child;

  @override
  State<RuneRouterFrameImplementation> createState() =>
      _RuneRouterFrameImplementationState();
}

class _RuneRouterFrameImplementationState
    extends State<RuneRouterFrameImplementation> {
  final _playbackControllerKey = GlobalKey();

  @override
  void initState() {
    super.initState();
  }

  @override
  Widget build(BuildContext context) {
    if ($router.path == '/') {
      return widget.child;
    }

    final library = Provider.of<LibraryPathProvider>(context);
    final r = Provider.of<ResponsiveProvider>(context);
    final crash = Provider.of<CrashProvider>(context);

    final isCar = r.smallerOrEqualTo(DeviceType.car, false);
    final isZune = r.smallerOrEqualTo(DeviceType.zune, false);
    final diskOnRight = r.smallerOrEqualTo(DeviceType.car, false);

    final showDisk = isZune || isCar;

    if (crash.report != null) {
      return Bsod(report: crash.report!);
    }

    if (library.currentPath == null) {
      return Container();
    }

    final mainContent = FocusTraversalOrder(
      order: const NumericFocusOrder(2),
      child: widget.child,
    );

    final path = Provider.of<RouterPathProvider>(context).path;
    final isSpecialLayout = isCoverArtWallLayout(path);

    // Create the PlaybackController with a key once
    final playbackController = KeyedSubtree(
      key: _playbackControllerKey,
      child: const FocusTraversalOrder(
        order: NumericFocusOrder(3),
        child: PlaybackController(),
      ),
    );

    return DropTarget(
      onDragDone: (detail) {
        final files = detail.files
            .map((xfile) => FastFilePickerPath.fromPath(xfile.name, xfile.path))
            .toList();

        safeOperatePlaybackWithMixQuery(
          context: context,
          queries: QueryList(),
          playbackMode: 99,
          hintPosition: -1,
          initialPlaybackId: -1,
          instantlyPlay: true,
          operateMode: PlaylistOperateMode.replace,
          fallbackPlayingItems: filterAudioFiles(
            files,
          ).map(PlayingItem.independentFile).toList(),
        );
      },
      child: Stack(
        children: [
          if (!disableBrandingAnimation) const BrandingAnimation(),
          ScaleFadeContainer(
            delay: disableBrandingAnimation
                ? const Duration(milliseconds: 0)
                : const Duration(milliseconds: 4350),
            duration: disableBrandingAnimation
                ? const Duration(milliseconds: 200)
                : const Duration(milliseconds: 500),
            child: FlipAnimationContext(
              child: FocusTraversalGroup(
                policy: OrderedTraversalPolicy(),
                child: RuneStack(
                  alignment: diskOnRight
                      ? Alignment.centerRight
                      : Alignment.bottomCenter,
                  children: [
                    if (isSpecialLayout && !showDisk) mainContent,
                    // Always keep the PlaybackController in the widget tree
                    // but make it visible only when needed
                    if (!showDisk)
                      Visibility(
                        visible: true, // Always in the tree
                        maintainState: true, // Keep the state
                        child: playbackController,
                      ),
                    FocusTraversalOrder(
                      order: const NumericFocusOrder(1),
                      child: DeviceTypeBuilder(
                        deviceType: const [
                          DeviceType.band,
                          DeviceType.dock,
                          DeviceType.tv,
                        ],
                        builder: (context, activeBreakpoint) {
                          final isSmallView =
                              activeBreakpoint == DeviceType.band ||
                              activeBreakpoint == DeviceType.dock;

                          if (!isSmallView) {
                            return NavigationBar(path: path);
                          }

                          return const Positioned(
                            top: -12,
                            left: -12,
                            child: NavigationBackButton(),
                          );
                        },
                      ),
                    ),
                    if (!(isSpecialLayout && !showDisk)) mainContent,
                    if (showDisk)
                      const BlockHitTestStack(
                        child: FocusTraversalOrder(
                          order: NumericFocusOrder(4),
                          child: CoverArtDisk(),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
