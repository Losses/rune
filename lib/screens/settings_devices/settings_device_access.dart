import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';
import '../../utils/api/device_grants.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/dialogs/information/error.dart';
import '../../utils/l10n.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/settings_page_padding.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/settings/settings_box_toggle.dart';
import '../../widgets/settings/settings_tile_title.dart';

class SettingsDeviceAccessPage extends StatefulWidget {
  const SettingsDeviceAccessPage({
    super.key,
    required this.client,
    this.loadGrants,
    this.saveGrants,
  });

  final ClientSummary client;
  final Future<GetDeviceGrantsResponse> Function(String fingerprint)?
  loadGrants;
  final Future<void> Function(SetDeviceGrantsRequest request)? saveGrants;

  @override
  State<SettingsDeviceAccessPage> createState() =>
      _SettingsDeviceAccessPageState();
}

class _SettingsDeviceAccessPageState extends State<SettingsDeviceAccessPage> {
  GetDeviceGrantsResponse? data;
  bool saving = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final result = await (widget.loadGrants ?? getDeviceGrants)(
        widget.client.fingerprint,
      );
      if (mounted) setState(() => data = result);
    } catch (error) {
      if (mounted) _showError(error);
    }
  }

  Future<void> _save({bool? control, DeviceLibraryGrant? library}) async {
    setState(() => saving = true);
    try {
      await (widget.saveGrants ?? setDeviceGrants)(
        SetDeviceGrantsRequest(
          fingerprint: widget.client.fingerprint,
          control: control,
          libraryGrants: library == null ? [] : [library],
        ),
      );
      await _load();
    } catch (error) {
      if (mounted) await _showError(error);
    } finally {
      if (mounted) setState(() => saving = false);
    }
  }

  Future<void> _showError(Object error) => showErrorDialog(
    context: context,
    title: S.of(context).unknownError,
    errorMessage: requestErrorText(S.of(context), error),
  );

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final current = data;
    return PageContentFrame(
      builder: (context) => SettingsPagePadding(
        child: SingleChildScrollView(
          padding: getScrollContainerPadding(context),
          child: SettingsBodyPadding(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  widget.client.alias,
                  style: FluentTheme.of(context).typography.title,
                ),
                if (current != null) ...[
                  SettingsBoxToggle(
                    title: s.controlPlayback,
                    subtitle: s.controlPlaybackDescription,
                    value: current.grants.control,
                    onChanged: saving ? null : (value) => _save(control: value),
                  ),
                  Padding(
                    padding: const EdgeInsets.only(top: 24, bottom: 8),
                    child: Text(
                      s.deviceAccessLibraries,
                      style: FluentTheme.of(context).typography.subtitle,
                    ),
                  ),
                  _LibraryAccessHeader(
                    title: '',
                    connection: s.deviceConnection,
                    sync: s.deviceSync,
                  ),
                  for (final library in current.libraries)
                    _LibraryAccessRow(
                      alias: library.alias,
                      connection: library.connection,
                      sync: library.sync,
                      saving: saving,
                      onConnectionChanged: (value) => _save(
                        library: DeviceLibraryGrant(
                          libraryId: library.libraryId,
                          connection: value,
                          sync: library.sync,
                        ),
                      ),
                      onSyncChanged: (value) => _save(
                        library: DeviceLibraryGrant(
                          libraryId: library.libraryId,
                          connection: library.connection,
                          sync: value,
                        ),
                      ),
                    ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _LibraryAccessHeader extends StatelessWidget {
  const _LibraryAccessHeader({
    required this.title,
    required this.connection,
    required this.sync,
  });

  final String title;
  final String connection;
  final String sync;

  @override
  Widget build(BuildContext context) => SmallerOrEqualToScreenSize(
    maxSize: 650,
    builder: (context, isNarrow) => isNarrow
        ? const SizedBox.shrink()
        : Padding(
            padding: const EdgeInsets.symmetric(vertical: 4),
            child: Row(
              children: [
                Expanded(child: Text(title)),
                Expanded(child: Text(connection)),
                Expanded(child: Text(sync)),
              ],
            ),
          ),
  );
}

class _LibraryAccessRow extends StatelessWidget {
  const _LibraryAccessRow({
    required this.alias,
    required this.connection,
    required this.sync,
    required this.saving,
    required this.onConnectionChanged,
    required this.onSyncChanged,
  });

  final String alias;
  final DeviceConnectionLevel connection;
  final DeviceSyncLevel sync;
  final bool saving;
  final ValueChanged<DeviceConnectionLevel> onConnectionChanged;
  final ValueChanged<DeviceSyncLevel> onSyncChanged;

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return SmallerOrEqualToScreenSize(
      maxSize: 650,
      builder: (context, isNarrow) {
        final connectionBox = ComboBox<DeviceConnectionLevel>(
          value: connection,
          items: [
            ComboBoxItem(
              value: DeviceConnectionLevel.none,
              child: Text(s.deviceConnectionOff),
            ),
            ComboBoxItem(
              value: DeviceConnectionLevel.view,
              child: Text(s.deviceConnectionView),
            ),
            ComboBoxItem(
              value: DeviceConnectionLevel.viewEdit,
              child: Text(s.deviceConnectionViewEdit),
            ),
          ],
          onChanged: saving
              ? null
              : (value) {
                  if (value != null && value != connection) {
                    onConnectionChanged(value);
                  }
                },
        );
        final syncBox = ComboBox<DeviceSyncLevel>(
          value: sync,
          items: [
            ComboBoxItem(
              value: DeviceSyncLevel.none,
              child: Text(s.deviceSyncOff),
            ),
            ComboBoxItem(
              value: DeviceSyncLevel.pull,
              child: Text(s.deviceSyncPull),
            ),
            ComboBoxItem(
              value: DeviceSyncLevel.pullPush,
              child: Text(s.deviceSyncPullPush),
            ),
          ],
          onChanged: saving
              ? null
              : (value) {
                  if (value != null && value != sync) onSyncChanged(value);
                },
        );
        final controls = Column(
          children: [
            Align(
              alignment: Alignment.centerLeft,
              child: Text(s.deviceConnection),
            ),
            connectionBox,
            const SizedBox(height: 8),
            Align(alignment: Alignment.centerLeft, child: Text(s.deviceSync)),
            syncBox,
          ],
        );
        return ListTile(
          title: isNarrow
              ? Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    SettingsTileTitle(
                      icon: Symbols.library_music,
                      title: alias,
                      subtitle: '',
                      showActions: false,
                      actionsBuilder: (_) => const SizedBox.shrink(),
                    ),
                    const SizedBox(height: 8),
                    controls,
                  ],
                )
              : Row(
                  children: [
                    Expanded(
                      child: SettingsTileTitle(
                        icon: Symbols.library_music,
                        title: alias,
                        subtitle: '',
                        showActions: false,
                        actionsBuilder: (_) => const SizedBox.shrink(),
                      ),
                    ),
                    Expanded(child: connectionBox),
                    const SizedBox(width: 8),
                    Expanded(child: syncBox),
                  ],
                ),
        );
      },
    );
  }
}
