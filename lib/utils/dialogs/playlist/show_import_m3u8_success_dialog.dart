import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../l10n.dart';

import '../information/error.dart';

/// Shows the entries an import could not match or matched ambiguously.
///
/// Does nothing when every entry matched exactly one track.
Future<void> showM3u8ImportResultDialog(
  BuildContext context,
  CreateM3u8PlaylistResponse result,
) async {
  if (result.notFoundPaths.isEmpty && result.ambiguousPaths.isEmpty) return;

  await showCreateImportM3u8SuccessDialog(
    context,
    notFoundPaths: result.notFoundPaths,
    ambiguousPaths: result.ambiguousPaths,
  );
}

/// Shows the partial-success dialog for an M3U8 import that needs attention.
///
/// [notFoundPaths] lists entries that matched no library track.
/// [ambiguousPaths] lists entries that matched more than one track; the import
/// selected one match for each. Each non-empty list gets its own localized
/// heading.
Future<void> showCreateImportM3u8SuccessDialog(
  BuildContext context, {
  required List<String> notFoundPaths,
  required List<String> ambiguousPaths,
}) async {
  final s = S.of(context);
  final hasAmbiguousPaths = ambiguousPaths.isNotEmpty;
  final errorMessage = hasAmbiguousPaths
      ? [
          '${s.importM3u8AmbiguousPaths}:\n${ambiguousPaths.join('\n\n')}',
          if (notFoundPaths.isNotEmpty)
            '${s.importM3u8NotFoundPaths}:\n${notFoundPaths.join('\n\n')}',
        ].join('\n\n')
      : notFoundPaths.join('\n\n');

  await showErrorDialog(
    context: context,
    title: s.importM3u8Success,
    subtitle: hasAmbiguousPaths
        ? s.importM3u8AmbiguousSubtitle
        : s.importM3u8SuccessSubtitle,
    errorMessage: errorMessage,
    useFilledButton: false,
    barrierDismissible: true,
  );
}
