import 'dart:math';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../utils/l10n.dart';
import '../../utils/settings_manager.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/api/system_info.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../utils/dialogs/register/show_register_dialog.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/no_shortcuts.dart';
import '../../widgets/tile/fancy_cover.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../bindings/bindings.dart';
import '../../providers/license.dart';
import '../../providers/responsive_providers.dart';

import '../settings_home/settings_home.dart';

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

class _LogoSection extends StatefulWidget {
  @override
  State<_LogoSection> createState() => _LogoSectionState();
}

class _LogoSectionState extends State<_LogoSection> {
  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      deviceType: DeviceType.tablet,
      builder: (context, isMini) {
        return Column(
          crossAxisAlignment:
              isMini ? CrossAxisAlignment.start : CrossAxisAlignment.center,
          children: [
            ContextMenuWrapper(
              contextAttachKey: _contextAttachKey,
              contextController: _contextController,
              onContextMenu: (offset) async {
                final triggeredMysterious =
                    await SettingsManager().getValue<bool>(SettingsHomePage.mysteriousKey);

                if (triggeredMysterious == true) return;
                if (!context.mounted) return;

                final targetContext = _contextAttachKey.currentContext;

                if (targetContext == null) return;
                final box = targetContext.findRenderObject() as RenderBox;

                if (!mounted) return;
                final position = box.localToGlobal(
                  offset,
                  ancestor: Navigator.of(context).context.findRenderObject(),
                );

                _contextController.showFlyout(
                  position: position,
                  builder: (context) {
                    return MenuFlyout(
                      items: [
                        MenuFlyoutItem(
                          leading: const Icon(Symbols.controller_gen),
                          text: Text(S.of(context).mysteriousButton),
                          onPressed: () {
                            SettingsManager().setValue(SettingsHomePage.mysteriousKey, true);

                            $showModal<void>(
                              context,
                              (context, $close) => NoShortcuts(
                                ContentDialog(
                                  title:
                                      Text(S.of(context).mysteriousModalTitle),
                                  content: Column(
                                    mainAxisSize: MainAxisSize.min,
                                    children: [
                                      Text(
                                        S.of(context).mysteriousModalContent,
                                        style: TextStyle(height: 1.4),
                                      ),
                                    ],
                                  ),
                                  actions: [
                                    Button(
                                      onPressed: () {
                                        $close(null);
                                      },
                                      child: Text(S.of(context).close),
                                    )
                                  ],
                                ),
                              ),
                              barrierDismissible: true,
                              dismissWithEsc: true,
                            );
                          },
                        ),
                      ],
                    );
                  },
                );
              },
              onMiddleClick: (_) {},
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 220),
                child: const Device(),
              ),
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
      title: S.of(context).system,
      rows: [
        [S.of(context).operatingSystem, data?.systemName ?? ""],
        [S.of(context).systemVersion, data?.systemOsVersion ?? ""],
        [S.of(context).kernelVersion, data?.systemKernelVersion ?? ""],
        [S.of(context).hostName, data?.systemHostName ?? ""],
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
      title: S.of(context).player,
      rows: [
        [S.of(context).buildHash, data?.buildSha.substring(0, 8) ?? ""],
        [S.of(context).buildDate, data?.buildDate ?? ""],
        [
          S.of(context).commitDate,
          data?.buildCommitTimestamp.split("T")[0] ?? ""
        ],
        [S.of(context).rustcVersion, data?.buildRustcSemver ?? ""],
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
    final license = Provider.of<LicenseProvider>(context);
    final theme = FluentTheme.of(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(S.of(context).activation,
            style: FluentTheme.of(context).typography.subtitle),
        const SizedBox(height: 4),
        Text(
          license.isPro
              ? S.of(context).runeIsActivated
              : S.of(context).evaluationMode,
        ),
        const SizedBox(height: 4),
        license.isPro
            ? Text(S.of(context).youMayBeAVictimOfGenuineSoftware)
            : HyperlinkButton(
                style: ButtonStyle(
                  padding: WidgetStateProperty.all(EdgeInsets.all(0)),
                  backgroundColor: WidgetStateProperty.all(Colors.transparent),
                  textStyle: WidgetStateProperty.resolveWith(
                    (states) {
                      if (states.isHovered) {
                        return TextStyle(
                          fontWeight: FontWeight.w600,
                          color: theme.resources.textOnAccentFillColorPrimary,
                        );
                      }
                      return TextStyle(
                        fontWeight: FontWeight.w600,
                        color: theme.resources.textOnAccentFillColorSecondary,
                      );
                    },
                  ),
                ),
                child: Text(S.of(context).considerPurchase),
                onPressed: () => showRegisterDialog(context),
              ),
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
        Text(S.of(context).copyright,
            style: FluentTheme.of(context).typography.subtitle),
        const SizedBox(height: 4),
        Text(S.of(context).copyrightAnnouncement),
        const SizedBox(height: 4),
        Text(S.of(context).licenseAnnouncement),
      ],
    );
  }
}

class Device extends StatefulWidget {
  const Device({super.key});

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

    return Listener(
      onPointerUp: (_) {
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
                    S.of(context).runePlayer,
                    S.of(context).axiomDesign,
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
