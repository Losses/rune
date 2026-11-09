import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../l10n/app_localizations.dart';
import '../../../utils/api/library_registry.dart';
import '../../../utils/api/renderers.dart';
import '../../../utils/api/request_failure.dart';
import '../../../utils/l10n.dart';
import '../../../providers/renderer_attachment.dart';
import '../../../providers/playlist.dart';
import '../../../providers/library_open_state.dart';
import '../../../utils/playing_item.dart';
import '../../../widgets/rune_clickable.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/settings/settings_tile_title.dart';

typedef RendererLoader =
    Future<List<RendererCandidate>> Function(List<String> libraryIds);
typedef RendererHandoff = Future<void> Function(String? target);
typedef RendererCast = Future<void> Function(RendererCandidate candidate);
typedef LibraryNamesLoader = Future<Map<String, String>> Function();

Future<Map<String, String>> _loadLibraryNames() async {
  try {
    final libraries = await listLocalLibraries();
    return {
      for (final library in libraries.libraries)
        library.libraryId: library.alias,
    };
  } catch (_) {
    return const {};
  }
}

Future<void> showPlayOnDialog(
  BuildContext context, {
  RendererLoader loader = listRenderers,
  RendererHandoff handoff = handoffRenderer,
  LibraryNamesLoader loadLibraryNames = _loadLibraryNames,
  List<String>? castLibraryIds,
  RendererCast? cast,
}) async {
  final playlist = Provider.of<PlaylistProvider>(context, listen: false);
  final attachment = Provider.of<RendererAttachmentProvider>(
    context,
    listen: false,
  );
  final libraryIds =
      castLibraryIds ??
      queueLibraryIds(playlist.items.map((entry) => entry.item));
  final namesFuture = loadLibraryNames();
  await showDialog<void>(
    context: context,
    builder: (dialogContext) => _PlayOnDialog(
      loader: loader,
      handoff: handoff,
      libraryNamesFuture: namesFuture,
      libraryIds: libraryIds,
      attachment: attachment,
      castMode: cast != null,
      cast: cast,
    ),
  );
}

Future<void> showCastOnDialog(
  BuildContext context, {
  required String libraryId,
  required RendererCast cast,
  RendererLoader loader = listRenderers,
  LibraryNamesLoader loadLibraryNames = _loadLibraryNames,
}) async {
  await showPlayOnDialog(
    context,
    loader: loader,
    loadLibraryNames: loadLibraryNames,
    castLibraryIds: [libraryId],
    cast: cast,
  );
}

/// Returns queue library ids in first-seen order, omitting non-library items.
List<String> queueLibraryIds(Iterable<PlayingItem> items) {
  final seen = <String>{};
  return [
    for (final item in items)
      if (item.libraryId case final id? when seen.add(id)) id,
  ];
}

class _PlayOnDialog extends StatefulWidget {
  const _PlayOnDialog({
    required this.loader,
    required this.handoff,
    required this.libraryNamesFuture,
    required this.libraryIds,
    required this.attachment,
    required this.castMode,
    required this.cast,
  });

  final RendererLoader loader;
  final RendererHandoff handoff;
  final Future<Map<String, String>> libraryNamesFuture;
  final List<String> libraryIds;
  final RendererAttachmentProvider attachment;
  final bool castMode;
  final RendererCast? cast;

  @override
  State<_PlayOnDialog> createState() => _PlayOnDialogState();
}

class _PlayOnDialogState extends State<_PlayOnDialog> {
  late Future<List<RendererCandidate>> _renderers;
  late Future<Map<String, String>> _libraryNames;
  String? _working;
  Object? _error;
  String? _errorAlias;
  String? _selectedFingerprint;
  bool _loadFailed = false;

  @override
  void initState() {
    super.initState();
    _load();
    _libraryNames = widget.libraryNamesFuture;
  }

  void _load() {
    _error = null;
    _errorAlias = null;
    _loadFailed = false;
    _renderers = widget.loader(widget.libraryIds);
  }

  Future<void> _select(
    String key,
    Future<void> Function() action, {
    String? alias,
  }) async {
    setState(() {
      _working = key;
      _error = null;
      _errorAlias = null;
    });
    try {
      await action();
      if (mounted) Navigator.of(context).pop();
    } on RequestFailure catch (error) {
      if (mounted) {
        setState(() {
          _error = error;
          _errorAlias = alias;
        });
      }
    } finally {
      if (mounted && _working != null) setState(() => _working = null);
    }
  }

  String _reason(
    AppLocalizations strings,
    RendererCandidate candidate,
    Map<String, String> libraryNames,
  ) => switch (candidate.availability) {
    RendererAvailability.offline => strings.playOnOffline,
    RendererAvailability.notApproved => strings.playOnNotApproved,
    RendererAvailability.noControl => strings.playOnNoControl,
    RendererAvailability.cantPlay => strings.playOnCantPlay(
      libraryNames[candidate.blockedLibraryId] ??
          candidate.blockedLibraryId ??
          '',
    ),
    RendererAvailability.available => '',
  };

  @override
  Widget build(BuildContext context) {
    final strings = S.of(context);
    final currentRenderer = widget.attachment.renderer;
    final thisDeviceSelected = currentRenderer == null;
    final loadFailed = _loadFailed;
    return NoShortcuts(
      ContentDialog(
        title: Column(children: [SizedBox(height: 8), Text(strings.playOn)]),
        content: SizedBox(
          width: 420,
          child: FutureBuilder<Map<String, String>>(
            future: _libraryNames,
            builder: (context, namesSnapshot) =>
                FutureBuilder<List<RendererCandidate>>(
                  future: _renderers,
                  builder: (context, snapshot) {
                    if (snapshot.hasError) {
                      if (!_loadFailed) {
                        WidgetsBinding.instance.addPostFrameCallback((_) {
                          if (mounted) setState(() => _loadFailed = true);
                        });
                      }
                      return Text(requestErrorText(strings, snapshot.error!));
                    }
                    if (!snapshot.hasData || !namesSnapshot.hasData) {
                      return const Center(child: ProgressRing());
                    }
                    final names = namesSnapshot.data!;
                    if (widget.castMode && snapshot.data!.isEmpty) {
                      return Center(child: Text(strings.noDevicesFound));
                    }
                    return Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        if (_error != null) ...[
                          Text(
                            _errorText(strings, _error!, _errorAlias, names),
                          ),
                          const SizedBox(height: 8),
                        ],
                        if (!widget.castMode)
                          _row(
                            fingerprint: 'this-device',
                            alias: strings.thisDevice,
                            subtitle: '',
                            selected: _selectedFingerprint == 'this-device',
                            playing: thisDeviceSelected,
                            enabled: currentRenderer != null,
                            working: _working == 'this-device',
                            onAction: () => _select(
                              'this-device',
                              () => widget.handoff(null),
                            ),
                          ),
                        for (final candidate in snapshot.data!)
                          _row(
                            fingerprint: candidate.fingerprint,
                            alias: candidate.alias,
                            subtitle:
                                candidate.availability ==
                                    RendererAvailability.available
                                ? ''
                                : _reason(strings, candidate, names),
                            selected:
                                _selectedFingerprint == candidate.fingerprint,
                            playing:
                                currentRenderer?.fingerprint ==
                                candidate.fingerprint,
                            enabled:
                                candidate.availability ==
                                    RendererAvailability.available &&
                                (widget.castMode || !candidate.attached),
                            working: _working == candidate.fingerprint,
                            onAction: () => _select(
                              candidate.fingerprint,
                              () => widget.castMode
                                  ? widget.cast!(candidate)
                                  : widget.handoff(candidate.fingerprint),
                              alias: candidate.alias,
                            ),
                          ),
                      ],
                    );
                  },
                ),
          ),
        ),
        actions: [
          if (loadFailed)
            Button(
              onPressed: () => setState(_load),
              child: Text(strings.playOnRetry),
            ),
          Button(
            child: Text(strings.close),
            onPressed: () => Navigator.pop(context),
          ),
        ],
      ),
    );
  }

  String _errorText(
    AppLocalizations strings,
    Object error,
    String? alias,
    Map<String, String> libraryNames,
  ) {
    if (alias != null && error is RequestFailure) {
      final cause = error.cause;
      if (cause case RequestFailureCauseCastNotPossible(:final libraryId)) {
        return strings.castNotPossible(
          alias,
          libraryNames[libraryId] ?? libraryId,
        );
      }
    }
    return requestErrorText(strings, error);
  }

  Widget _row({
    required String fingerprint,
    required String alias,
    required String subtitle,
    required bool selected,
    required bool playing,
    required bool enabled,
    required bool working,
    required VoidCallback onAction,
  }) => ListTile.selectable(
    title: SettingsTileTitle(
      icon: Symbols.cast,
      badgeContent: playing
          ? const Icon(Symbols.play_arrow, size: 12, color: Colors.white)
          : null,
      badgeColor: FluentTheme.of(context).accentColor,
      title: alias,
      subtitle: subtitle,
      showActions: selected,
      actionsBuilder: (context) => Row(
        children: [
          Button(
            onPressed: enabled && !working ? onAction : null,
            child: working ? const ProgressRing() : Text(S.of(context).playOn),
          ),
        ],
      ),
    ),
    selected: selected,
    onSelectionChange: (_) =>
        setState(() => _selectedFingerprint = fingerprint),
  );
}

class PlayOnButton extends StatelessWidget {
  const PlayOnButton({super.key});

  @override
  Widget build(BuildContext context) {
    final renderer = Provider.of<RendererAttachmentProvider>(context).renderer;
    final remoteLibrary = Provider.of<LibraryOpenStateProvider>(
      context,
    ).isRemoteControl;
    return RuneClickable(
      onPressed: remoteLibrary ? null : () => showPlayOnDialog(context),
      child: Opacity(
        opacity: remoteLibrary ? 0.48 : 1,
        child: Icon(
          Symbols.cast,
          color: renderer == null ? null : FluentTheme.of(context).accentColor,
        ),
      ),
    );
  }
}
