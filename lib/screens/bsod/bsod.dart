import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/providers/responsive_providers.dart';

class Bsod extends StatelessWidget {
  final String report;

  const Bsod({super.key, required this.report});

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.band,
        DeviceType.fish,
        DeviceType.dock,
        DeviceType.zune,
        DeviceType.mobile,
        DeviceType.tv
      ],
      builder: (context, deviceType) {
        double fontSizeFactor = 1;

        if (deviceType == DeviceType.zune) {
          fontSizeFactor = 0.8;
        }

        if (deviceType == DeviceType.dock || deviceType == DeviceType.band) {
          return Container(
            color: theme.accentColor.dark,
            child: Center(
              child: LayoutBuilder(builder: (context, constraints) {
                final size = min(constraints.maxHeight, constraints.maxWidth);
                return Icon(
                  Symbols.falling,
                  color: Colors.white,
                  size: size * 0.6,
                );
              }),
            ),
          );
        }

        return Container(
          color: theme.accentColor.dark,
          child: Center(
            child: SingleChildScrollView(
              child: Container(
                constraints: const BoxConstraints(maxWidth: 1200),
                padding: const EdgeInsets.all(12),
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    if (deviceType != DeviceType.fish)
                      Text(
                        "( ˘･з･)",
                        style: theme.typography.display?.apply(
                          fontSizeFactor: fontSizeFactor *
                              (deviceType == DeviceType.zune ? 0.6 : 1),
                        ),
                      ),
                    const SizedBox(height: 24),
                    Container(
                      constraints: const BoxConstraints(maxWidth: 500),
                      child: Text(
                        "Your player ran into a problem and needs a restart. We respect your privacy and won't collect any data.",
                        style: theme.typography.bodyLarge
                            ?.apply(
                              fontSizeFactor: 1.15 * fontSizeFactor,
                              fontWeightDelta: -5,
                            )
                            .merge(
                              const TextStyle(height: 1.4),
                            ),
                      ),
                    ),
                    const SizedBox(height: 16),
                    Container(
                      constraints: const BoxConstraints(maxWidth: 800),
                      child: Text(
                        report,
                        style: theme.typography.body
                            ?.apply(
                              fontSizeFactor: fontSizeFactor,
                            )
                            .merge(
                              const TextStyle(height: 1.5),
                            ),
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
}
