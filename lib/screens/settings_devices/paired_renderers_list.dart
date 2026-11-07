import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../bindings/bindings.dart';
import '../../utils/api/remove_trusted_server.dart';
import '../../utils/api/renderers.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/dialogs/information/confirm.dart';
import '../../utils/dialogs/information/error.dart';
import '../../utils/dialogs/play_on/play_on_dialog.dart';
import '../../utils/l10n.dart';

class PairedRenderersList extends StatefulWidget {
  const PairedRenderersList({
    super.key,
    this.loadRenderers = listRenderers,
    this.removeRenderer = removeTrustedServer,
    this.confirmRemove = _confirmRemove,
  });

  final Future<List<RendererCandidate>> Function(List<String>) loadRenderers;
  final Future<bool> Function(String) removeRenderer;
  final Future<bool?> Function(BuildContext) confirmRemove;

  static Future<bool?> _confirmRemove(BuildContext context) {
    final s = S.of(context);
    return showConfirmDialog(
      context: context,
      title: s.removeTrust,
      subtitle: s.removeTrustSubtitle,
      yesLabel: s.remove,
      noLabel: s.cancel,
    );
  }

  @override
  State<PairedRenderersList> createState() => _PairedRenderersListState();
}

class _PairedRenderersListState extends State<PairedRenderersList> {
  late Future<List<RendererCandidate>> _renderers;
  final Set<String> _removing = {};

  @override
  void initState() {
    super.initState();
    _load();
  }

  void _load() {
    _renderers = widget.loadRenderers(const []);
  }

  Future<void> _remove(String fingerprint) async {
    final s = S.of(context);
    final confirmed = await widget.confirmRemove(context);
    if (confirmed != true || !mounted) return;
    setState(() => _removing.add(fingerprint));
    try {
      await widget.removeRenderer(fingerprint);
      if (mounted) setState(_load);
    } catch (error) {
      if (!mounted) return;
      await showErrorDialog(
        context: context,
        title: s.unknownError,
        errorMessage: requestErrorText(s, error),
      );
    } finally {
      if (mounted) setState(() => _removing.remove(fingerprint));
    }
  }

  @override
  Widget build(BuildContext context) => FutureBuilder<List<RendererCandidate>>(
    future: _renderers,
    builder: (context, snapshot) {
      if (snapshot.hasError) {
        return Text(requestErrorText(S.of(context), snapshot.error!));
      }
      if (!snapshot.hasData) {
        return const Center(child: ProgressRing());
      }

      final strings = S.of(context);
      return Column(
        children: [
          for (final renderer in snapshot.data!)
            ListTile(
              leading: const Icon(Symbols.devices),
              title: Text(renderer.alias),
              subtitle: renderer.availability == RendererAvailability.available
                  ? null
                  : Text(rendererUnavailableReason(strings, renderer)),
              trailing: Button(
                onPressed: _removing.contains(renderer.fingerprint)
                    ? null
                    : () => _remove(renderer.fingerprint),
                child: _removing.contains(renderer.fingerprint)
                    ? const SizedBox.square(
                        dimension: 16,
                        child: ProgressRing(),
                      )
                    : Text(strings.remove),
              ),
            ),
        ],
      );
    },
  );
}
