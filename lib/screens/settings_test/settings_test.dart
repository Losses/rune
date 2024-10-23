import 'dart:math';
import 'dart:ui';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/svg.dart';

import '../../screens/settings_test/lens_flare.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

class _SettingsTestPageState extends State<SettingsTestPage> {
  @override
  Widget build(BuildContext context) {
    return const PageContentFrame(
      child: StaggerDemo(),
    );
  }
}

class StaggerAnimation extends StatelessWidget {
  StaggerAnimation({super.key, required this.controller})
      : diskOpacity = Tween<double>(
          begin: 0.0,
          end: 1.0,
        ).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              0.0,
              0.5,
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
              0.7,
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
              0.7,
              curve: Curves.ease,
            ),
          ),
        ),
        diskRotation = Tween<double>(begin: pi, end: 0.0).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              0.0,
              0.7,
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
              0.6,
              1.0,
              curve: Curves.ease,
            ),
          ),
        ),
        boxTranslate = Tween<double>(begin: 100.0, end: 10.0).animate(
          CurvedAnimation(
            parent: controller,
            curve: const Interval(
              0.6,
              1.0,
              curve: Curves.ease,
            ),
          ),
        );

  final Animation<double> controller;
  final Animation<double> diskBlur;
  final Animation<double> diskSize;
  final Animation<double> diskOpacity;
  final Animation<double> diskRotation;
  final Animation<double> boxOpacity;
  final Animation<double> boxTranslate;

  Widget _buildAnimation(BuildContext context, Widget? child) {
    final theme = FluentTheme.of(context);

    return Stack(
      alignment: Alignment.center,
      children: [
        const LensFlareEffect(),
        SizedBox(
          width: 360,
          height: 360,
          child: ImageFiltered(
            imageFilter: ImageFilter.blur(
                sigmaX: diskBlur.value, sigmaY: diskBlur.value),
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
                    SvgPicture.asset('assets/disk-center.svg'),
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
      ],
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
      duration: const Duration(milliseconds: 2000),
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
      await _controller.forward().orCancel;
      // await _controller.reverse().orCancel;
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
