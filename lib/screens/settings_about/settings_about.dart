import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../../utils/settings_page_padding.dart';
import '../../utils/api/system_info.dart';
import '../../widgets/tile/fancy_cover.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../messages/system.pb.dart';
import '../../providers/responsive_providers.dart';

const size = 400.0;

class SettingsAboutPage extends StatelessWidget {
  const SettingsAboutPage({super.key});

  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: SmallerOrEqualTo(
        deviceType: DeviceType.tablet,
        builder: (context, isMini) {
          if (isMini) {
            return SingleChildScrollView(
              padding: getScrollContainerPadding(context),
              child: SettingsPagePadding(
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      _LogoSection(),
                      FutureBuilder<SystemInfoResponse>(
                        future: systemInfo(),
                        builder: (context, snapshot) =>
                            _InfoSection(data: snapshot.data),
                      ),
                    ],
                  ),
                ),
              ),
            );
          }

          return Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.max,
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(48, 0, 24, 0),
                child: _LogoSection(),
              ),
              Expanded(
                child: FutureBuilder<SystemInfoResponse>(
                  future: systemInfo(),
                  builder: (context, snapshot) =>
                      _InfoSection(data: snapshot.data),
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}

class _LogoSection extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      deviceType: DeviceType.tablet,
      builder: (context, isMini) {
        return Column(
          crossAxisAlignment:
              isMini ? CrossAxisAlignment.start : CrossAxisAlignment.center,
          children: [
            ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 220),
              child: const Device(),
            ),
            const SizedBox(height: 24),
            SvgPicture.asset(
              'assets/mono_color_logo.svg',
              width: 128,
              colorFilter: ColorFilter.mode(
                FluentTheme.of(context).inactiveColor,
                BlendMode.srcIn,
              ),
            ),
            const SizedBox(height: 28),
          ],
        );
      },
    );
  }
}

class _InfoSection extends StatelessWidget {
  final SystemInfoResponse? data;

  const _InfoSection({this.data});

  @override
  Widget build(BuildContext context) {
    return SmoothHorizontalScroll(
      builder: (context, controller) {
        return SmallerOrEqualTo(
          deviceType: DeviceType.tablet,
          builder: (context, isMini) {
            if (isMini) {
              return Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  _BuildInfo(data: data),
                  SystemInfo(data: data),
                  _ActivationInfo(),
                  _CopyrightInfo(),
                ],
              );
            }
            return SingleChildScrollView(
              controller: controller,
              scrollDirection: Axis.horizontal,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 12),
                child: Wrap(
                  direction: Axis.vertical,
                  spacing: 8,
                  runSpacing: 24,
                  children: [
                    _BuildInfo(data: data),
                    SystemInfo(data: data),
                    _ActivationInfo(),
                    _CopyrightInfo(),
                  ],
                ),
              ),
            );
          },
        );
      },
    );
  }
}

class SystemInfo extends StatelessWidget {
  const SystemInfo({
    super.key,
    required this.data,
  });

  final SystemInfoResponse? data;

  @override
  Widget build(BuildContext context) {
    return _InfoTable(
      title: "System",
      rows: [
        ["Operating system", data?.systemName ?? ""],
        ["System Version", data?.systemOsVersion ?? ""],
        ["Kernel Version", data?.systemKernelVersion ?? ""],
        ["Host Name", data?.systemHostName ?? ""],
      ],
    );
  }
}

class _BuildInfo extends StatelessWidget {
  const _BuildInfo({
    required this.data,
  });

  final SystemInfoResponse? data;

  @override
  Widget build(BuildContext context) {
    return _InfoTable(
      title: "Player",
      rows: [
        ["Build Hash", data?.buildSha.substring(0, 8) ?? ""],
        ["Build Date", data?.buildDate ?? ""],
        ["Commit Date", data?.buildCommitTimestamp.split("T")[0] ?? ""],
        ["Rustc version", data?.buildRustcSemver ?? ""],
      ],
    );
  }
}

class _InfoTable extends StatelessWidget {
  final String title;
  final List<List<String>> rows;

  const _InfoTable({required this.title, required this.rows});

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(title, style: FluentTheme.of(context).typography.subtitle),
        const SizedBox(height: 4),
        Table(
          columnWidths: const {
            0: IntrinsicColumnWidth(),
            1: IntrinsicColumnWidth(),
            2: FixedColumnWidth(8),
            3: IntrinsicColumnWidth(),
          },
          defaultVerticalAlignment: TableCellVerticalAlignment.middle,
          children: rows
              .map((row) => TableRow(
                    children: [
                      const SizedBox(height: 20),
                      Text(row[0]),
                      const SizedBox(),
                      Text(row[1]),
                    ],
                  ))
              .toList(),
        ),
        const SizedBox(height: 12),
      ],
    );
  }
}

class _ActivationInfo extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text("Activation", style: FluentTheme.of(context).typography.subtitle),
        const SizedBox(height: 4),
        const Text("Rune is activated"),
        const SizedBox(height: 4),
        const Row(
          children: [
            Text("Product ID"),
            SizedBox(width: 12),
            Text("DG8FV-B9TKY-FRT9J-6CRCC-XPQ4G"),
          ],
        ),
        const SizedBox(height: 4),
        const Text("You may be a victim of genuine software."),
        const SizedBox(height: 12),
      ],
    );
  }
}

class _CopyrightInfo extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text("Copyright", style: FluentTheme.of(context).typography.subtitle),
        const SizedBox(height: 4),
        const Text('Copyright © 2024 Rune Player Developers.'),
        const SizedBox(height: 4),
        const Text('This product is licensed under the MPL license.'),
      ],
    );
  }
}

class Device extends StatefulWidget {
  const Device({
    super.key,
  });

  @override
  State<Device> createState() => _DeviceState();
}

class _DeviceState extends State<Device> {
  int configIndex = 0;
  int colorHash = 0;
  Random random = Random();

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return GestureDetector(
      onTap: () {
        setState(() {
          configIndex = random.nextInt(9);
          colorHash = random.nextInt(100);
        });
      },
      child: FocusableActionDetector(
        child: AspectRatio(
          aspectRatio: (5360 / 2814),
          child: Stack(
            alignment: Alignment.center,
            children: [
              SvgPicture.asset(
                'assets/device-layer-1.svg',
                width: size,
                colorFilter: ColorFilter.mode(
                  theme.accentColor.normal,
                  BlendMode.srcIn,
                ),
              ),
              SvgPicture.asset(
                'assets/device-layer-2.svg',
                width: size,
              ),
              LayoutBuilder(builder: (context, constraints) {
                return FancyCover(
                  size: constraints.maxWidth * 0.41,
                  ratio: 9 / 16,
                  texts: (
                    "Rune Player",
                    "Axiom Design",
                    "Version 0.0.5-dev",
                  ),
                  colorHash: colorHash,
                  configIndex: configIndex,
                );
              }),
              SvgPicture.asset(
                'assets/device-layer-3.svg',
                width: size,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
