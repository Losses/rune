import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/library_registry.dart';
import '../../../utils/library_registry_messages.dart';
import '../../../utils/api/library_registry.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../utils/l10n.dart';
import '../../../utils/library_ui/library_ui_state.dart';
import '../../../utils/router/navigation.dart';

class PeerLibrariesDialog extends StatefulWidget {
  final ClientSummary client;
  final void Function() close;
  const PeerLibrariesDialog({
    super.key,
    required this.client,
    required this.close,
  });

  @override
  State<PeerLibrariesDialog> createState() => _PeerLibrariesDialogState();
}

class _PeerLibrariesDialogState extends State<PeerLibrariesDialog> {
  ListPeerLibrariesResponse? response;
  bool failed = false;
  final paths = <String, String>{};
  final errors = <String, String>{};
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
      final result = await listPeerLibraries(widget.client.fingerprint);
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
    } catch (_) {
      if (mounted && response == null) setState(() => failed = true);
    }
  }

  Future<void> _add(PeerLibrary library) async {
    setState(() {
      busy.add(library.libraryId);
      errors.remove(library.libraryId);
    });
    try {
      final result = await addRemoteLibrary(library, widget.client.fingerprint);
      if (!mounted) return;
      if (result.error != null) {
        setState(() => errors[library.libraryId] = result.error!);
      } else {
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
        if (!mounted) return;
        if (record == null) return;
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
      }
    } catch (error) {
      if (mounted) setState(() => errors[library.libraryId] = error.toString());
    } finally {
      if (mounted) setState(() => busy.remove(library.libraryId));
    }
  }

  Future<void> _open(String path) async {
    final registry = Provider.of<LibraryRegistryProvider>(
      context,
      listen: false,
    );
    try {
      await registry.open(path);
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
    return ContentDialog(
      title: Text(s.deviceLibraries),
      content: failed
          ? Text(s.unknownError)
          : switch (state.status) {
              PeerLibrariesStatus.loading => const Center(
                child: ProgressRing(),
              ),
              PeerLibrariesStatus.error => Text(state.error!),
              PeerLibrariesStatus.empty => Text(s.noSharedLibraries),
              PeerLibrariesStatus.rows => SingleChildScrollView(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    for (final library in state.libraries)
                      Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Row(
                            children: [
                              Expanded(child: Text(library.alias)),
                              if (library.added) ...[
                                Text(s.sharedLibraryAdded),
                                if (paths[library.libraryId]
                                    case final path?) ...[
                                  const SizedBox(width: 12),
                                  Button(
                                    onPressed: () => _open(path),
                                    child: Text(s.openSharedLibrary),
                                  ),
                                ],
                              ] else
                                Button(
                                  onPressed: busy.contains(library.libraryId)
                                      ? null
                                      : () => _add(library),
                                  child: Text(s.addSharedLibrary),
                                ),
                            ],
                          ),
                          if (library.sync == DeviceSyncLevel.pull)
                            Text(s.readOnly),
                          if (errors[library.libraryId] case final error?)
                            Text(error),
                        ],
                      ),
                  ],
                ),
              ),
            },
      actions: [Button(onPressed: widget.close, child: Text(s.close))],
    );
  }
}
