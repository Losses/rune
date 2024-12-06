import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';
import 'package:flutter_staggered_grid_view/flutter_staggered_grid_view.dart';

import '../../utils/l10n.dart';
import '../../utils/router/navigation.dart';
import '../../widgets/rune_icon_button.dart';
import '../../widgets/router/rune_stack.dart';
import '../../providers/responsive_providers.dart';

import 'widgets/settings/branding_animation_settings.dart';
import 'widgets/settings/cover_wall_richness_settings.dart';
import 'widgets/settings/library_cover_wallpaper_settings.dart';

class SettingsLaboratory extends StatelessWidget {
  const SettingsLaboratory({super.key});

  @override
  Widget build(BuildContext context) {
    final typography = FluentTheme.of(context).typography;

    return RuneStack(
      children: [
        _buildBackButton(),
        _buildTitle(context, typography),
        _buildSettingsGrid(context),
      ],
    );
  }

  Widget _buildBackButton() {
    return Positioned(
      top: 16,
      left: 16,
      child: RuneIconButton(
        icon: Icon(Symbols.arrow_back, size: 24),
        onPressed: () => $pop(),
      ),
    );
  }

  Widget _buildTitle(BuildContext context, Typography typography) {
    return Align(
      alignment: Alignment.topCenter,
      child: Padding(
        padding: EdgeInsets.only(top: 20),
        child: Text(
          S.of(context).laboratory,
          style: typography.title,
        ),
      ),
    );
  }

  Widget _buildSettingsGrid(BuildContext context) {
    return Align(
      alignment: Alignment.topCenter,
      child: Container(
        padding: EdgeInsets.symmetric(horizontal: 8.0),
        constraints: BoxConstraints(maxWidth: 800),
        child: Padding(
          padding: EdgeInsets.only(top: 68),
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
        ],
      ),
    );
  }
}
