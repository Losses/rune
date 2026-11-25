import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';
import 'package:provider/provider.dart';

import '../../../utils/l10n.dart';
import '../../../utils/ax_shadow.dart';
import '../../../utils/dialogs/add_library/add_library_guide.dart';
import '../../../widgets/band_screen_fallback_button.dart';
import '../../../providers/responsive_providers.dart';

class WelcomePage extends StatelessWidget {
  const WelcomePage({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final r = Provider.of<ResponsiveProvider>(context);

    if (r.smallerOrEqualTo(DeviceType.dock, false)) {
      return Center(
        child: BandScreenFallbackButton(
          icon: Icon(Symbols.folder_open),
          onPressed: () => showAddLibraryGuide(context),
        ),
      );
    }

    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 400, maxHeight: 560),
        child: Container(
          decoration: BoxDecoration(
            color: theme.cardColor,
            borderRadius: BorderRadius.circular(3),
            boxShadow: axShadow(20),
          ),
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.start,
              children: [
                Expanded(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.center,
                    children: [
                      SvgPicture.asset(
                        'assets/mono_color_logo.svg',
                        colorFilter: ColorFilter.mode(
                          theme.inactiveColor,
                          BlendMode.srcIn,
                        ),
                      ),
                      r.smallerOrEqualTo(DeviceType.car, false)
                          ? r.smallerOrEqualTo(DeviceType.band, false)
                                ? const SizedBox(height: 2)
                                : const SizedBox(height: 14)
                          : const SizedBox(height: 20),
                      Column(
                        children: [
                          if (!r.smallerOrEqualTo(DeviceType.band, false) &&
                              !r.smallerOrEqualTo(DeviceType.zune, false))
                            Padding(
                              padding: const EdgeInsets.symmetric(
                                horizontal: 24,
                              ),
                              child: Text(
                                S.of(context).selectLibraryDirectorySubtitle,
                                textAlign: TextAlign.center,
                                style: TextStyle(height: 1.4),
                              ),
                            ),
                          r.smallerOrEqualTo(DeviceType.car, false)
                              ? const SizedBox(height: 20)
                              : r.smallerOrEqualTo(DeviceType.zune, false)
                              ? const SizedBox(height: 12)
                              : const SizedBox(height: 56),
                          FilledButton(
                            child: Text(S.of(context).addLibrary),
                            onPressed: () => showAddLibraryGuide(context),
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
                if (!r.smallerOrEqualTo(DeviceType.belt, false))
                  Text(
                    S.of(context).copyrightAnnouncement,
                    textAlign: TextAlign.center,
                    style: theme.typography.caption?.apply(
                      color: theme.inactiveColor.withAlpha(80),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
