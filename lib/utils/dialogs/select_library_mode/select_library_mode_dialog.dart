import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/no_shortcuts.dart';

import '../../l10n.dart';

import 'library_mode_option.dart';

class SelectLibraryModeDialog extends StatelessWidget {
  final Function(String?) onClose;

  const SelectLibraryModeDialog({super.key, required this.onClose});

  List<LibraryModeOption> _getModeOptions(BuildContext context) => [
        LibraryModeOption(
          title: S.of(context).portableMode,
          subtitle: S.of(context).portableModeSubtitle,
          value: "Portable",
        ),
        LibraryModeOption(
          title: S.of(context).localMode,
          subtitle: S.of(context).localModeSubtitle,
          value: "Redirected",
        ),
      ];

  @override
  Widget build(BuildContext context) {
    return NoShortcuts(
      ContentDialog(
        title: Column(
          children: [
            const SizedBox(height: 8),
            Text(S.of(context).libraryMode),
          ],
        ),
        constraints: const BoxConstraints(
          maxWidth: 386.0,
          maxHeight: 756.0,
        ),
        content: _DialogContent(
          options: _getModeOptions(context),
          onOptionSelected: onClose,
        ),
        actions: [
          Button(
            child: Text(S.of(context).cancel),
            onPressed: () => onClose(null),
          ),
        ],
      ),
    );
  }
}

class _DialogContent extends StatelessWidget {
  final List<LibraryModeOption> options;
  final Function(String?) onOptionSelected;

  const _DialogContent({
    required this.options,
    required this.onOptionSelected,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          S.of(context).libraryModeSubtitle,
          style: const TextStyle(height: 1.4),
        ),
        const SizedBox(height: 12),
        ...options.map((option) => _buildOptionButton(context, option)).expand(
              (widget) => [widget, const SizedBox(height: 8)],
            ),
      ],
    );
  }

  Widget _buildOptionButton(BuildContext context, LibraryModeOption option) {
    final theme = FluentTheme.of(context);

    return Button(
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 4),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.start,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              option.title,
              style: theme.typography.body?.apply(fontSizeFactor: 1.1),
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.start,
            ),
            const SizedBox(height: 4),
            Text(
              option.subtitle,
              style: theme.typography.caption?.apply(
                color: theme.inactiveColor.withAlpha(160),
              ),
              textAlign: TextAlign.start,
            ),
          ],
        ),
      ),
      onPressed: () => onOptionSelected(option.value),
    );
  }
}
