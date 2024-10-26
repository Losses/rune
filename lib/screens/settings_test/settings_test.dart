import 'dart:io';
import 'dart:math';
import 'dart:ui';

import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/svg.dart';
import 'package:path_provider/path_provider.dart';

import '../../utils/api/sfx_play.dart';
import '../../screens/settings_test/lens_flare.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';

class AssetHelper {
  AssetHelper._privateConstructor();

  static final AssetHelper _instance = AssetHelper._privateConstructor();

  static AssetHelper get instance => _instance;

  Future<File> getAudioFileFromAssets(String asset) async {
    final byteData = await rootBundle.load(asset);
    final tempFile = File(
        "${(await getTemporaryDirectory()).path}/${asset.split('/').last}");
    final file = await tempFile.writeAsBytes(
      byteData.buffer
          .asUint8List(byteData.offsetInBytes, byteData.lengthInBytes),
    );
    return file;
  }
}

Future<File> startSfxFile =
    AssetHelper.instance.getAudioFileFromAssets('assets/startup_1.ogg');

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

class _SettingsTestPageState extends State<SettingsTestPage> {
  @override
  Widget build(BuildContext context) {
    return const PageContentFrame(
      top: false,
      bottom: false,
      left: false,
      right: false,
      child: StaggerDemo(),
    );
  }
}

const frame1 = 2.5 / 4.5; // 0.0 ~ 2.5, Rotating disk
const frame2 = 3.0 / 4.5; // 2.5 ~ 3.0, Box appearing
const frame3 = 3.5 / 4.5; // 3.0 ~ 3.5, Zoom out and text fade in
const frame4 = 4.0 / 4.5; // 4.0 ~ 4.5, Logo fade out
const frame5 = 4.5 / 4.5;

double mapFlareAlpha(double x, double a) {
  if (x <= a) {
    return x / a;
  } else if (x <= 1 - a) {
    return 1.0;
  } else {
    return (1 - x) / a;
  }
}

class StaggerAnimation extends StatelessWidget {
  StaggerAnimation({super.key, required this.controller})
      : flareX = Tween<double>(
          begin: -1.0,
          end: 1.0,
        ).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              0.0,
              frame1,
              curve: Curves.ease,
            ),
          ),
        ),
        diskOpacity = Tween<double>(
          begin: 0.0,
          end: 1.0,
        ).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              0.0,
              frame1,
              curve: Curves.ease,
            ),
          ),
        ),
        diskBlur = Tween<double>(
          begin: 40.0,
          end: 0.0,
        ).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              0.0,
              frame1,
              curve: Curves.ease,
            ),
          ),
        ),
        diskSize = Tween<double>(
          begin: 2,
          end: 0.7,
        ).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              0.0,
              frame1,
              curve: Curves.ease,
            ),
          ),
        ),
        diskRotation = Tween<double>(begin: pi, end: 0.0).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              0.0,
              frame1,
              curve: Curves.ease,
            ),
          ),
        ),
        boxOpacity = Tween<double>(
          begin: 0.0,
          end: 1.0,
        ).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              frame1 - 0.1,
              frame2,
              curve: Curves.ease,
            ),
          ),
        ),
        boxTranslate = Tween<double>(begin: 100.0, end: 10.0).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              frame1 - 0.1,
              frame2,
              curve: Curves.ease,
            ),
          ),
        ),
        logoSize = Tween<double>(begin: 1.0, end: 0.75).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              frame2 - 0.1,
              frame3,
              curve: Curves.ease,
            ),
          ),
        ),
        logoTranslate = Tween<double>(begin: 0, end: -60).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              frame2 - 0.1,
              frame3,
              curve: Curves.ease,
            ),
          ),
        ),
        textOpacity = Tween<double>(begin: 0.0, end: 1.0).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              frame2,
              frame3,
              curve: Curves.ease,
            ),
          ),
        ),
        textTranslate = Tween<double>(begin: 190, end: 170).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              frame2,
              frame3,
              curve: Curves.ease,
            ),
          ),
        ),
        totalOpacity = Tween<double>(begin: 1.0, end: 0.0).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              frame3,
              frame4,
              curve: Curves.ease,
            ),
          ),
        ),
        totalSize = Tween<double>(begin: 1, end: 0.9).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              frame3,
              frame4,
              curve: Curves.ease,
            ),
          ),
        );

  final Animation<double> controller;
  final Animation<double> flareX;
  final Animation<double> diskBlur;
  final Animation<double> diskSize;
  final Animation<double> diskOpacity;
  final Animation<double> diskRotation;
  final Animation<double> boxOpacity;
  final Animation<double> boxTranslate;
  final Animation<double> logoSize;
  final Animation<double> logoTranslate;
  final Animation<double> textOpacity;
  final Animation<double> textTranslate;
  final Animation<double> totalOpacity;
  final Animation<double> totalSize;

  Widget _buildAnimation(BuildContext context, Widget? child) {
    final theme = FluentTheme.of(context);

    return LayoutBuilder(
      builder: (context, constraints) {
        return Opacity(
          opacity: totalOpacity.value,
          child: Transform.scale(
            scale: totalSize.value,
            child: LensFlareEffect(
              alpha: mapFlareAlpha((flareX.value + 1) / 2, 0.1),
              flarePosition: Offset(
                constraints.maxWidth / 2 + 160 * flareX.value,
                constraints.maxHeight / 2,
              ),
              child: Center(
                child: Stack(
                  children: [
                    Transform(
                      transform: Matrix4.identity()
                        ..setEntry(3, 2, 0.001)
                        ..scale(logoSize.value)
                        ..translate(0.0, logoTranslate.value, 0.0),
                      alignment: Alignment.center,
                      child: Stack(
                        alignment: Alignment.center,
                        children: [
                          SizedBox(
                            width: 360,
                            height: 360,
                            child: ImageFiltered(
                              imageFilter: ImageFilter.blur(
                                sigmaX: diskBlur.value,
                                sigmaY: diskBlur.value,
                              ),
                              child: Transform(
                                transform: Matrix4.identity()
                                  ..setEntry(3, 2, 0.001)
                                  ..scale(diskSize.value)
                                  ..rotateZ(diskRotation.value),
                                alignment: Alignment.center,
                                child: Opacity(
                                  opacity: diskOpacity.value,
                                  child: Stack(
                                    children: [
                                      SvgPicture.asset(
                                        'assets/disk-border.svg',
                                        colorFilter: ColorFilter.mode(
                                          theme.accentColor,
                                          BlendMode.srcIn,
                                        ),
                                      ),
                                      SvgPicture.asset(
                                          'assets/disk-center.svg'),
                                    ],
                                  ),
                                ),
                              ),
                            ),
                          ),
                          SizedBox(
                            width: 300,
                            height: 300,
                            child: Transform(
                              transform: Matrix4.identity()
                                ..setEntry(3, 2, 0.001)
                                ..translate(boxTranslate.value),
                              alignment: Alignment.center,
                              child: Opacity(
                                opacity: boxOpacity.value,
                                child: ShaderMask(
                                  blendMode: BlendMode.dstIn,
                                  shaderCallback: (Rect bounds) {
                                    return LinearGradient(
                                      colors: <Color>[
                                        Colors.black.withAlpha(240),
                                        Colors.black,
                                        Colors.black
                                      ],
                                    ).createShader(bounds);
                                  },
                                  child: SvgPicture.asset('assets/box.svg'),
                                ),
                              ),
                            ),
                          ),
                          Opacity(
                            opacity: textOpacity.value,
                            child: Transform.translate(
                              offset: Offset(0, textTranslate.value),
                              child: SvgPicture.asset(
                                'assets/branding-text.svg',
                                width: 160,
                                colorFilter: ColorFilter.mode(
                                  FluentTheme.of(context).inactiveColor,
                                  BlendMode.srcIn,
                                ),
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      builder: _buildAnimation,
      animation: controller,
    );
  }
}

class StaggerDemo extends StatefulWidget {
  const StaggerDemo({super.key});

  @override
  State<StaggerDemo> createState() => _StaggerDemoState();
}

class _StaggerDemoState extends State<StaggerDemo>
    with TickerProviderStateMixin {
  late AnimationController _controller;

  @override
  void initState() {
    super.initState();

    _controller = AnimationController(
      duration: const Duration(milliseconds: 4500),
      vsync: this,
    );
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _playAnimation() async {
    try {
      _controller.reset();
      sfxPlay((await startSfxFile).path);
      await _controller.forward().orCancel;
    } on TickerCanceled {
      // The animation got canceled, probably because we were disposed.
    }
  }

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () {
        _playAnimation();
      },
      child: Center(
        child: StaggerAnimation(controller: _controller.view),
      ),
    );
  }
}
