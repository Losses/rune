import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../l10n/app_localizations.dart';
import '../../../providers/broadcast.dart';
import '../../../providers/library_registry.dart';
import '../../../utils/api/device_grants.dart';
import '../../../utils/api/library_registry.dart';
import '../../../utils/api/request_failure.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/l10n.dart';
import '../../../utils/library_registry_messages.dart';
import '../../../utils/library_ui/library_ui_state.dart';
import '../../../utils/router/navigation.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/settings/settings_tile_title.dart';
import '../../../screens/settings_neighbors/utils/show_add_remote_device_dialog.dart';
import '../../../screens/settings_server/widgets/peer_libraries_dialog.dart';
import '../../add_library_flows.dart';

enum AddLibrarySource { localFolder, runeDevice, webDav, s3 }

class AddLibraryGuideDependencies {
  const AddLibraryGuideDependencies({
    this.searchNeighbors,
    this.addNeighborManually,
    this.pairedDevices,
    this.loadLibraries,
    this.loadGrants,
    this.addLibrary,
    this.registerControl,
    this.registerOpenDal,
    this.openRecord,
  });

  final Future<void> Function(BuildContext)? searchNeighbors;
  final Future<bool?> Function(BuildContext)? addNeighborManually;
  final List<ClientSummary>? pairedDevices;
  final PeerLibrariesLoader? loadLibraries;
  final Future<GetDeviceGrantsResponse> Function(String)? loadGrants;
  final PeerLibraryAdder? addLibrary;
  final Future<RegisterLibraryResponse> Function(String, String)?
  registerControl;
  final Future<RegisterLibraryResponse> Function(OpenDalLibraryService, String)?
  registerOpenDal;
  final Future<void> Function(String)? openRecord;
}

Future<void> showAddLibraryGuide(
  BuildContext context, {
  bool navigateIfFailed = false,
  Future<void> Function(BuildContext)? addLocalFolder,
  AddLibraryGuideDependencies dependencies =
      const AddLibraryGuideDependencies(),
}) async {
  final source = await $showModal<AddLibrarySource>(
    context,
    (context, $close) =>
        _AddLibraryGuideDialog($close: $close, dependencies: dependencies),
  );
  if (!context.mounted || source == null) return;

  if (source != AddLibrarySource.localFolder) return;
  await (addLocalFolder ??
      (context) => addLocalFolderLibrary(
        context,
        navigateIfFailed: navigateIfFailed,
      ))(context);
}

class _AddLibraryGuideDialog extends StatefulWidget {
  const _AddLibraryGuideDialog({
    required this.$close,
    required this.dependencies,
  });

  final void Function(AddLibrarySource?) $close;
  final AddLibraryGuideDependencies dependencies;

  @override
  State<_AddLibraryGuideDialog> createState() => _AddLibraryGuideDialogState();
}

class _AddLibraryGuideDialogState extends State<_AddLibraryGuideDialog> {
  AddLibrarySource? _selected;
  ClientSummary? _device;
  int _step = 1;
  ListPeerLibrariesResponse? _libraries;
  bool? _canControl;
  String? _selectedChoice;
  String? _selectedLibraryId;
  bool _loading = false;
  bool _registeringOpenDal = false;
  final _openDalForm = _OpenDalFormController();
  final _busy = <String>{};

  @override
  void dispose() {
    _openDalForm.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return NoShortcuts(
      ContentDialog(
        title: Column(children: [SizedBox(height: 8), Text(s.addLibrary)]),
        content: switch (_step) {
          1 => _sourceContent(s),
          2 =>
            _selected == AddLibrarySource.webDav
                ? _webDavContent(s)
                : _selected == AddLibrarySource.s3
                ? _s3Content(s)
                : _deviceContent(s),
          _ => _libraryContent(s),
        },
        actions: [
          if (_step > 1)
            Button(
              onPressed: () => setState(() {
                _step = _step - 1;
                _selectedLibraryId = null;
              }),
              child: Text(s.back),
            ),
          Button(onPressed: () => widget.$close(null), child: Text(s.cancel)),
          if (_step == 2 &&
              (_selected == AddLibrarySource.webDav ||
                  _selected == AddLibrarySource.s3))
            Button(
              onPressed: _canSubmitOpenDal && !_registeringOpenDal
                  ? _registerOpenDal
                  : null,
              child: Text(s.confirm),
            ),
        ],
      ),
    );
  }

  Widget _sourceContent(AppLocalizations s) => Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      _sourceRow(
        AddLibrarySource.localFolder,
        Symbols.folder,
        s.localFolder,
        s.localFolderSubtitle,
      ),
      _sourceRow(
        AddLibrarySource.runeDevice,
        Symbols.devices,
        s.runeDevice,
        s.runeDeviceSubtitle,
      ),
      _sourceRow(
        AddLibrarySource.webDav,
        Symbols.cloud_sync,
        s.webDav,
        s.webDavSubtitle,
      ),
      _sourceRow(AddLibrarySource.s3, Symbols.cloud_sync, s.s3, s.s3Subtitle),
    ],
  );

  bool get _canSubmitOpenDal {
    final fields = _selected == AddLibrarySource.webDav
        ? [
            _openDalForm.server,
            _openDalForm.folder,
            _openDalForm.username,
            _openDalForm.password,
          ]
        : [
            _openDalForm.server,
            _openDalForm.bucket,
            _openDalForm.region,
            _openDalForm.folder,
            _openDalForm.accessKeyId,
            _openDalForm.secretAccessKey,
          ];
    return fields.every((value) => value.trim().isNotEmpty);
  }

  Widget _webDavContent(AppLocalizations s) => _openDalFormContent(
    fields: [
      (s.server, _openDalForm.serverController),
      (s.folder, _openDalForm.folderController),
      (s.username, _openDalForm.usernameController),
      (s.password, _openDalForm.passwordController),
    ],
    passwordFields: {_openDalForm.passwordController},
  );

  Widget _s3Content(AppLocalizations s) => _openDalFormContent(
    fields: [
      (s.server, _openDalForm.serverController),
      (s.bucket, _openDalForm.bucketController),
      (s.region, _openDalForm.regionController),
      (s.folder, _openDalForm.folderController),
      (s.accessKeyId, _openDalForm.accessKeyIdController),
      (s.secretAccessKey, _openDalForm.secretAccessKeyController),
    ],
    passwordFields: {_openDalForm.secretAccessKeyController},
  );

  Widget _openDalFormContent({
    required List<(String, TextEditingController)> fields,
    required Set<TextEditingController> passwordFields,
  }) => Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      for (var index = 0; index < fields.length; index++) ...[
        if (index > 0) const SizedBox(height: 16),
        InfoLabel(
          label: fields[index].$1,
          child: TextBox(
            controller: fields[index].$2,
            obscureText: passwordFields.contains(fields[index].$2),
            onChanged: (_) => setState(() {}),
          ),
        ),
      ],
    ],
  );

  Future<void> _registerOpenDal() async {
    final folder = _openDalForm.folder.trim();
    final bucket = _openDalForm.bucket.trim();
    final aliasSource = _selected == AddLibrarySource.webDav ? folder : bucket;
    final aliasParts = aliasSource.split('/').where((part) => part.isNotEmpty);
    final alias = aliasParts.isEmpty
        ? _openDalForm.server.trim()
        : aliasParts.last;
    final service = _selected == AddLibrarySource.webDav
        ? OpenDalLibraryServiceWebDav(
            endpoint: _openDalForm.server.trim(),
            root: folder,
            username: _openDalForm.username.trim(),
            password: _openDalForm.password,
          )
        : OpenDalLibraryServiceS3(
            endpoint: _openDalForm.server.trim(),
            bucket: bucket,
            region: _openDalForm.region.trim(),
            root: folder,
            accessKeyId: _openDalForm.accessKeyId.trim(),
            secretAccessKey: _openDalForm.secretAccessKey,
          );
    setState(() {
      _registeringOpenDal = true;
    });
    try {
      final response =
          await (widget.dependencies.registerOpenDal ?? registerOpenDalLibrary)(
            service,
            alias,
          );
      final outcome = response.outcome;
      if (outcome case RegisterLibraryOutcomeFailed(:final value)) {
        throw LibraryRegistryFailure(value, response.detail ?? '');
      }
      final record = response.record;
      if (record == null) throw StateError('Registration returned no record');
      await _openRecord(record.recordId);
    } catch (error) {
      if (!mounted) return;
      await showErrorDialog(
        context: context,
        title: S.of(context).addLibrary,
        errorMessage: error is LibraryRegistryFailure
            ? libraryRegistryFailureMessage(S.of(context), error.kind)
            : S.of(context).unknownError,
      );
    } finally {
      if (mounted) setState(() => _registeringOpenDal = false);
    }
  }

  Widget _deviceContent(AppLocalizations s) {
    final users =
        widget.dependencies.pairedDevices ??
        Provider.of<BroadcastProvider>(context).users;
    final devices = users
        .where((client) => client.status == ClientStatus.approved)
        .toList();
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 420, maxHeight: 420),
      child: ListView(
        shrinkWrap: true,
        children: [
          for (final device in devices)
            _choiceRow(
              'device:${device.fingerprint}',
              Symbols.devices,
              device.alias,
              device.deviceModel,
              () => _loadDevice(device),
            ),
          _choiceRow(
            'search',
            Symbols.search,
            s.searchNeighbors,
            s.searchNeighborsSubtitle,
            () async {
              await (widget.dependencies.searchNeighbors ??
                  (context) => pairRuneDevice(context))(context);
              if (mounted) {
                Provider.of<BroadcastProvider>(
                  context,
                  listen: false,
                ).fetchUsers();
              }
            },
          ),
          _choiceRow(
            'manual',
            Symbols.add,
            s.addNeighborManually,
            s.addNeighborManuallySubtitle,
            () async {
              final paired =
                  await (widget.dependencies.addNeighborManually ??
                      (context) =>
                          showAddRemoteDeviceDialog(true, context))(context);
              if (paired == true && mounted) {
                Provider.of<BroadcastProvider>(
                  context,
                  listen: false,
                ).fetchUsers();
              }
            },
          ),
        ],
      ),
    );
  }

  /// A device-step row in the same selectable shape as the source rows.
  Widget _choiceRow(
    String key,
    IconData icon,
    String title,
    String subtitle,
    VoidCallback onConfirm,
  ) {
    final selected = _selectedChoice == key;
    return ListTile.selectable(
      title: SettingsTileTitle(
        icon: icon,
        title: title,
        subtitle: subtitle,
        showActions: selected,
        actionsBuilder: (context) => Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            if (selected)
              Button(onPressed: onConfirm, child: Text(S.of(context).confirm)),
          ],
        ),
      ),
      selected: selected,
      onSelectionChange: (value) =>
          setState(() => _selectedChoice = value ? key : null),
    );
  }

  Widget _libraryContent(AppLocalizations s) {
    if (_loading) {
      return const Center(child: ProgressRing());
    }
    final response = _libraries;
    final state = response == null ? null : peerLibrariesState(s, response);
    if (state == null || state.status == PeerLibrariesStatus.error) {
      return Text(state?.error ?? s.unknownError);
    }
    if (state.status == PeerLibrariesStatus.empty) {
      return Text(s.noSharedLibraries);
    }
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 420, maxHeight: 420),
      child: ListView.builder(
        shrinkWrap: true,
        itemCount: response!.libraries.length,
        itemBuilder: (context, index) {
          final library = response.libraries[index];
          final selected = _selectedLibraryId == library.libraryId;
          final canSync = library.sync != DeviceSyncLevel.none;
          final canControl = _canControl == true;
          // One reason only: the one that disables an action, else read-only.
          final subtitle = !canSync
              ? s.libraryNotShared
              : !canControl
              ? s.controlPermissionRequired
              : library.sync == DeviceSyncLevel.pull
              ? s.readOnly
              : '';
          return PeerLibraryRow(
            library: library,
            selected: selected,
            subtitle: subtitle,
            onSelectionChange: (_) =>
                setState(() => _selectedLibraryId = library.libraryId),
            actionsBuilder: (_) => Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Button(
                  onPressed: !canSync || _busy.contains(library.libraryId)
                      ? null
                      : () => _sync(library),
                  child: Text(s.syncToThisDevice),
                ),
                const SizedBox(height: 8),
                Button(
                  onPressed: !canControl || _busy.contains(library.libraryId)
                      ? null
                      : () => _control(),
                  child: Text(s.controlPlayback),
                ),
              ],
            ),
          );
        },
      ),
    );
  }

  Future<void> _loadDevice(ClientSummary device) async {
    setState(() {
      _device = device;
      _step = 3;
      _loading = true;
      _libraries = null;
      _canControl = false;
    });
    try {
      final results = await Future.wait([
        (widget.dependencies.loadLibraries ?? listPeerLibraries)(
          device.fingerprint,
        ),
        (widget.dependencies.loadGrants ?? getDeviceGrants)(device.fingerprint),
      ]);
      if (!mounted) return;
      setState(() {
        _libraries = results[0] as ListPeerLibrariesResponse;
        _canControl = (results[1] as GetDeviceGrantsResponse).grants.control;
        _loading = false;
      });
    } catch (_) {
      if (mounted) setState(() => _loading = false);
    }
  }

  Future<void> _sync(PeerLibrary library) async {
    final device = _device!;
    setState(() => _busy.add(library.libraryId));
    try {
      final result = await (widget.dependencies.addLibrary ?? addRemoteLibrary)(
        library,
        device.fingerprint,
      );
      if (result.error != null) {
        throw RequestFailure(
          'AddRemoteLibraryRequest',
          RequestErrorKind.internal,
          result.error!,
        );
      }
      await _openRecord(result.recordId);
    } catch (error) {
      if (mounted) {
        await showErrorDialog(
          context: context,
          title: S.of(context).addLibrary,
          errorMessage: requestErrorText(S.of(context), error),
        );
      }
    } finally {
      if (mounted) setState(() => _busy.remove(library.libraryId));
    }
  }

  Future<void> _control() async {
    final device = _device!;
    try {
      final registry = Provider.of<LibraryRegistryProvider>(
        context,
        listen: false,
      );
      final result =
          await (widget.dependencies.registerControl ??
              registry.registerRemoteControl)(device.fingerprint, device.alias);
      if (result.record != null) await _openRecord(result.record!.recordId);
    } catch (error) {
      if (mounted) {
        await showErrorDialog(
          context: context,
          title: S.of(context).addLibrary,
          errorMessage: error is LibraryRegistryFailure
              ? libraryRegistryFailureMessage(S.of(context), error.kind)
              : requestErrorText(S.of(context), error),
        );
      }
    }
  }

  Future<void> _openRecord(String recordId) async {
    final registry = Provider.of<LibraryRegistryProvider>(
      context,
      listen: false,
    );
    try {
      await (widget.dependencies.openRecord ?? registry.open)(recordId);
    } catch (error) {
      if (!mounted) return;
      await showFailedToInitializeLibrary(
        context,
        error is LibraryRegistryFailure
            ? libraryRegistryFailureMessage(S.of(context), error.kind)
            : S.of(context).unknownError,
      );
      return;
    }
    widget.$close(null);
    $push('/library');
  }

  Widget _sourceRow(
    AddLibrarySource source,
    IconData icon,
    String title,
    String subtitle,
  ) {
    final selected = _selected == source;
    return ListTile.selectable(
      title: SettingsTileTitle(
        icon: icon,
        title: title,
        subtitle: subtitle,
        showActions: selected,
        actionsBuilder: (context) => Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            if (selected)
              Button(
                onPressed: () {
                  if (source == AddLibrarySource.runeDevice ||
                      source == AddLibrarySource.webDav ||
                      source == AddLibrarySource.s3) {
                    setState(() => _step = 2);
                  } else {
                    widget.$close(source);
                  }
                },
                child: Text(S.of(context).confirm),
              ),
          ],
        ),
      ),
      selected: selected,
      onSelectionChange: (value) =>
          setState(() => _selected = value ? source : null),
    );
  }
}

class _OpenDalFormController {
  final serverController = TextEditingController();
  final folderController = TextEditingController();
  final usernameController = TextEditingController();
  final passwordController = TextEditingController();
  final bucketController = TextEditingController();
  final regionController = TextEditingController();
  final accessKeyIdController = TextEditingController();
  final secretAccessKeyController = TextEditingController();

  String get server => serverController.text;
  String get folder => folderController.text;
  String get username => usernameController.text;
  String get password => passwordController.text;
  String get bucket => bucketController.text;
  String get region => regionController.text;
  String get accessKeyId => accessKeyIdController.text;
  String get secretAccessKey => secretAccessKeyController.text;

  void dispose() {
    serverController.dispose();
    folderController.dispose();
    usernameController.dispose();
    passwordController.dispose();
    bucketController.dispose();
    regionController.dispose();
    accessKeyIdController.dispose();
    secretAccessKeyController.dispose();
  }
}
