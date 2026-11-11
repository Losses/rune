import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/library_registry.dart';
import '../../../utils/library_registry_messages.dart';
import '../../../utils/api/library_registry.dart';
import '../../../utils/api/request_failure.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../utils/dialogs/subsonic/subsonic_relogin_dialog.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/l10n.dart';
import '../../../utils/library_ui/library_ui_state.dart';
import '../../../utils/router/navigation.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/settings/settings_tile_title.dart';

typedef PeerLibrariesLoader =
    Future<ListPeerLibrariesResponse> Function(String fingerprint);
typedef PeerLibraryAdder =
    Future<AddRemoteLibraryResponse> Function(
      PeerLibrary library,
      String fingerprint,
    );

class PeerLibraryRow extends StatelessWidget {
  const PeerLibraryRow({
    super.key,
    required this.library,
    required this.selected,
    required this.subtitle,
    required this.actionsBuilder,
    required this.onSelectionChange,
  });

  final PeerLibrary library;
  final bool selected;
  final String subtitle;
  final Widget Function(BuildContext) actionsBuilder;
  final ValueChanged<bool> onSelectionChange;

  @override
  Widget build(BuildContext context) => ListTile.selectable(
    title: SettingsTileTitle(
      icon: Symbols.library_music,
      title: library.alias,
      subtitle: subtitle,
      showActions: selected,
      actionsBuilder: actionsBuilder,
    ),
    selected: selected,
    onSelectionChange: onSelectionChange,
  );
}

class PeerLibrariesDialog extends StatefulWidget {
  final ClientSummary client;
  final void Function() close;
  final PeerLibrariesLoader loadLibraries;
  final PeerLibraryAdder addLibrary;

  const PeerLibrariesDialog({
    super.key,
    required this.client,
    required this.close,
    this.loadLibraries = listPeerLibraries,
    this.addLibrary = addRemoteLibrary,
  });

  @override
  State<PeerLibrariesDialog> createState() => _PeerLibrariesDialogState();
}

class _PeerLibrariesDialogState extends State<PeerLibrariesDialog> {
  ListPeerLibrariesResponse? response;
  bool failed = false;
  String? selectedLibraryId;
  final paths = <String, String>{};
  final busy = <String>{};

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final registry = Provider.of<LibraryRegistryProvider>(
      context,
      listen: false,
    );
    try {
      final result = await widget.loadLibraries(widget.client.fingerprint);
      if (mounted) setState(() => response = result);
      if (result.errorKind == null && result.libraries.any((e) => e.added)) {
        await registry.refresh();
        if (mounted) {
          setState(() {
            for (final record in registry.records) {
              if (record.kind case LibraryRecordKindSummaryReplica(
                :final libraryId,
                :final ownerFingerprint,
              )) {
                if (ownerFingerprint == widget.client.fingerprint) {
                  paths[libraryId] = record.recordId;
                }
              }
            }
          });
        }
      }
    } on RequestFailure catch (error) {
      if (mounted) {
        setState(() => failed = true);
        await _showError(error);
      }
    } catch (error) {
      if (mounted) {
        setState(() => failed = true);
        await _showError(error);
      }
    }
  }

  Future<void> _showError(Object error) => showErrorDialog(
    context: context,
    title: S.of(context).deviceLibraries,
    errorMessage: requestErrorText(S.of(context), error),
  );

  Future<void> _add(PeerLibrary library) async {
    setState(() => busy.add(library.libraryId));
    try {
      final result = await widget.addLibrary(
        library,
        widget.client.fingerprint,
      );
      if (!mounted) return;
      if (result.error != null) {
        await _showErrorMessage(result.error!);
        return;
      }
      final registry = Provider.of<LibraryRegistryProvider>(
        context,
        listen: false,
      );
      await registry.refresh();
      LibraryRecordSummary? record;
      for (final candidate in registry.records) {
        if (candidate.kind case LibraryRecordKindSummaryReplica(
          :final libraryId,
        ) when libraryId == library.libraryId) {
          record = candidate;
          break;
        }
      }
      if (!mounted || record == null) return;
      setState(() {
        paths[library.libraryId] = record!.recordId;
        response = response!.copyWith(
          libraries: response!.libraries
              .map(
                (e) => e.libraryId == library.libraryId
                    ? e.copyWith(added: true)
                    : e,
              )
              .toList(),
        );
      });
    } on RequestFailure catch (error) {
      if (mounted) await _showError(error);
    } catch (error) {
      if (mounted) await _showError(error);
    } finally {
      if (mounted) setState(() => busy.remove(library.libraryId));
    }
  }

  Future<void> _showErrorMessage(String error) => showErrorDialog(
    context: context,
    title: S.of(context).deviceLibraries,
    errorMessage: error,
  );

  Future<void> _open(String path) async {
    final registry = Provider.of<LibraryRegistryProvider>(
      context,
      listen: false,
    );
    try {
      await openLibraryWithSubsonicRelogin(context, registry, path);
    } catch (error) {
      if (!mounted) return;
      await showFailedToInitializeLibrary(
        context,
        error is LibraryRegistryFailure
            ? libraryRegistryFailureMessage(S.of(context), error.kind)
            : S.of(context).unknownError,
      );
      if (mounted) $$replace('/');
      return;
    }
    widget.close();
    $push('/library');
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final state = peerLibrariesState(s, response);
    return NoShortcuts(
      ContentDialog(
        title: Column(children: [SizedBox(height: 8), Text(s.deviceLibraries)]),
        content: failed
            ? Text(s.unknownError)
            : switch (state.status) {
                PeerLibrariesStatus.loading => const Center(
                  child: ProgressRing(),
                ),
                PeerLibrariesStatus.error => Text(state.error!),
                PeerLibrariesStatus.empty => Text(s.noSharedLibraries),
                PeerLibrariesStatus.rows => SizedBox(
                  width: 420,
                  height: 420,
                  child: ListView.builder(
                    itemCount: state.libraries.length,
                    itemBuilder: (context, index) {
                      final library = state.libraries[index];
                      final selected = selectedLibraryId == library.libraryId;
                      final path = paths[library.libraryId];
                      final subtitle = library.sync == DeviceSyncLevel.pull
                          ? s.readOnly
                          : library.added
                          ? s.sharedLibraryAdded
                          : '';
                      return PeerLibraryRow(
                        library: library,
                        selected: selected,
                        subtitle: subtitle,
                        onSelectionChange: (_) => setState(
                          () => selectedLibraryId = library.libraryId,
                        ),
                        actionsBuilder: (context) => Row(
                          children: [
                            if (library.added && path != null)
                              Button(
                                onPressed: () => _open(path),
                                child: Text(s.openSharedLibrary),
                              )
                            else
                              Button(
                                onPressed: busy.contains(library.libraryId)
                                    ? null
                                    : () => _add(library),
                                child: Text(s.addSharedLibrary),
                              ),
                          ],
                        ),
                      );
                    },
                  ),
                ),
              },
        actions: [Button(onPressed: widget.close, child: Text(s.close))],
      ),
    );
  }
}
