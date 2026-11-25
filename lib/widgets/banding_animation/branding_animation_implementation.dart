import 'dart:ui';
import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/svg.dart';
import 'package:vector_math/vector_math_64.dart' hide Colors;

import '../../widgets/banding_animation/widgets/map_flare_alpha.dart';

import 'lens_flare.dart';

const frame1 = 2.5 / 4.5; // 0.0 ~ 2.5, Rotating disk
const frame2 = 3.0 / 4.5; // 2.5 ~ 3.0, Box appearing
const frame3 = 3.5 / 4.5; // 3.0 ~ 3.5, Zoom out and text fade in
const frame4 = 4.0 / 4.5; // 4.0 ~ 4.5, Logo fade out
const frame5 = 4.5 / 4.5;

class BrandingAnimationImplementation extends StatelessWidget {
  BrandingAnimationImplementation({super.key, required this.controller})
    : flareX = Tween<double>(begin: -1.0, end: 1.0).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(0.0, frame1, curve: Curves.ease),
        ),
      ),
      diskOpacity = Tween<double>(begin: 0.0, end: 1.0).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(0.0, frame1, curve: Curves.ease),
        ),
      ),
      diskBlur = Tween<double>(begin: 40.0, end: 0.0).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(0.0, frame1, curve: Curves.ease),
        ),
      ),
      diskSize = Tween<double>(begin: 2, end: 0.7).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(0.0, frame1, curve: Curves.ease),
        ),
      ),
      diskRotation = Tween<double>(begin: pi, end: 0.0).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(0.0, frame1, curve: Curves.ease),
        ),
      ),
      boxOpacity = Tween<double>(begin: 0.0, end: 1.0).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(frame1 - 0.1, frame2, curve: Curves.ease),
        ),
      ),
      boxTranslate = Tween<double>(begin: 100.0, end: 10.0).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(frame1 - 0.1, frame2, curve: Curves.ease),
        ),
      ),
      logoSize = Tween<double>(begin: 1.0, end: 0.75).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(frame2 - 0.1, frame3, curve: Curves.ease),
        ),
      ),
      logoTranslate = Tween<double>(begin: 0, end: -60).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(frame2 - 0.1, frame3, curve: Curves.ease),
        ),
      ),
      textOpacity = Tween<double>(begin: 0.0, end: 1.0).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(frame2, frame3, curve: Curves.ease),
        ),
      ),
      textTranslate = Tween<double>(begin: 190, end: 170).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(frame2, frame3, curve: Curves.ease),
        ),
      ),
      totalOpacity = Tween<double>(begin: 1.0, end: 0.0).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(frame3, frame4, curve: Curves.ease),
        ),
      ),
      totalSize = Tween<double>(begin: 1, end: 0.9).animate(
        CurvedAnimation(
          parent: controller,
          curve: const Interval(frame3, frame4, curve: Curves.ease),
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
                        ..scaleByDouble(
                          logoSize.value,
                          logoSize.value,
                          logoSize.value,
                          1.0,
                        )
                        ..translateByVector3(
                          Vector3(0.0, logoTranslate.value, 0.0),
                        ),
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
                                  ..scaleByDouble(
                                    diskSize.value,
                                    diskSize.value,
                                    diskSize.value,
                                    1.0,
                                  )
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
                                        'assets/disk-center.svg',
                                      ),
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
                                ..translateByDouble(
                                  boxTranslate.value,
                                  0.0,
                                  0.0,
                                  1.0,
                                ),
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
                                        Colors.black,
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
    return AnimatedBuilder(builder: _buildAnimation, animation: controller);
  }
}
