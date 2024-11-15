import 'dart:math';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../widgets/action_buttons.dart';
import '../providers/responsive_providers.dart';
import '../generated/l10n.dart';

class NoItems extends StatelessWidget {
  final String title;
  final bool hasRecommendation;
  final VoidCallback reloadData;
  final bool userGenerated;

  const NoItems({
    super.key,
    required this.title,
    required this.hasRecommendation,
    required this.reloadData,
    this.userGenerated = false,
  });

  @override
  Widget build(BuildContext context) {
    final typography = FluentTheme.of(context).typography;

    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.band,
        DeviceType.dock,
        DeviceType.tv,
        DeviceType.station
      ],
      builder: (context, activeBreakpoint) {
        if (activeBreakpoint == DeviceType.band ||
            activeBreakpoint == DeviceType.dock) {
          final size = Provider.of<ScreenSizeProvider>(context);
          return Icon(
            Symbols.select,
            size: (min(size.screenSize.height, size.screenSize.width) * 0.8)
                .clamp(0, 48),
          );
        }
        return Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(Symbols.select, size: 48),
            const SizedBox(height: 8),
            Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.center,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  title,
                  style: typography.title,
                  textAlign: TextAlign.center,
                ),
                const SizedBox(height: 4),
                userGenerated
                    ? Text(
                        S.of(context).tryCreatingYourOwnCollection,
                        textAlign: TextAlign.center,
                      )
                    : hasRecommendation
                        ? Text(
                            S.of(context).theseActionsMayHelp,
                            textAlign: TextAlign.center,
                          )
                        : Text(
                            S.of(context).tryScanningNewFiles,
                            textAlign: TextAlign.center,
                          ),
              ],
            ),
            if (!userGenerated) ...[
              const SizedBox(height: 24),
              ActionButtons(
                reloadData: reloadData,
                hasRecommendation: hasRecommendation,
              ),
            ]
          ],
        );
      },
    );
  }
}
