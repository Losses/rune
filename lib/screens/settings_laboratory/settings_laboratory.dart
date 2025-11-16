import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';
import 'package:flutter_staggered_grid_view/flutter_staggered_grid_view.dart';

import '../../utils/l10n.dart';
import '../../utils/router/navigation.dart';
import '../../widgets/rune_clickable.dart';
import '../../widgets/router/rune_stack.dart';
import '../../providers/responsive_providers.dart';

import 'widgets/settings/cafe_mode_settings.dart';
import 'widgets/settings/force_zune_settings.dart';
import 'widgets/settings/branding_animation_settings.dart';
import 'widgets/settings/cover_wall_richness_settings.dart';
import 'widgets/settings/library_cover_wallpaper_settings.dart';
import 'widgets/settings/mild_spectrum_settings.dart';
import 'widgets/settings/tray_icon_color_mode_settings.dart';

class SettingsLaboratory extends StatelessWidget {
  const SettingsLaboratory({super.key});

  @override
  Widget build(BuildContext context) {
    final typography = FluentTheme.of(context).typography;

    final viewPadding = MediaQuery.of(context).viewPadding;

    return RuneStack(
      children: [
        _buildBackButton(viewPadding),
        _buildTitle(context, typography, viewPadding),
        _buildSettingsGrid(context, viewPadding),
      ],
    );
  }

  Widget _buildBackButton(EdgeInsets viewPadding) {
    return Positioned(
      top: 16 + viewPadding.top,
      left: 16 + viewPadding.left,
      child: RuneClickable(
        child: Icon(Symbols.arrow_back, size: 24),
        onPressed: () => $pop(),
      ),
    );
  }

  Widget _buildTitle(
    BuildContext context,
    Typography typography,
    EdgeInsets viewPadding,
  ) {
    return Align(
      alignment: Alignment.topCenter,
      child: Padding(
        padding: EdgeInsets.only(top: 20 + viewPadding.top),
        child: Text(
          S.of(context).laboratory,
          style: typography.title,
        ),
      ),
    );
  }

  Widget _buildSettingsGrid(
    BuildContext context,
    EdgeInsets viewPadding,
  ) {
    return Align(
      alignment: Alignment.topCenter,
      child: Container(
        padding: EdgeInsets.symmetric(horizontal: 8.0),
        constraints: BoxConstraints(maxWidth: 800),
        child: Padding(
          padding: EdgeInsets.only(top: 68 + viewPadding.top),
          child: _ResponsiveSettingsGrid(),
        ),
      ),
    );
  }
}

class _ResponsiveSettingsGrid extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      deviceType: DeviceType.phone,
      builder: (context, isMini) => MasonryGridView(
        padding: EdgeInsets.only(top: 4),
        gridDelegate: SliverSimpleGridDelegateWithFixedCrossAxisCount(
          crossAxisCount: isMini ? 1 : 2,
        ),
        mainAxisSpacing: 4,
        crossAxisSpacing: 4,
        children: [
          CoverWallRichnessSettings(),
          LibraryCoverWallpaperSettings(),
          BrandingAnimationSettings(),
          CafeModeSettings(),
          ForceZuneSettings(),
          MildSpectrumSettings(),
          TrayIconColorModeSettings(),
        ],
      ),
    );
  }
}
