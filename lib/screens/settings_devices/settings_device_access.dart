import 'package:fluent_ui/fluent_ui.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../utils/api/device_grants.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/dialogs/information/error.dart';
import '../../utils/l10n.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/settings_page_padding.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/settings/settings_box_combo_box.dart';
import '../../widgets/settings/settings_box_toggle.dart';
import '../../widgets/unavailable_page_on_band.dart';

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
    final current = data;
    if (current == null) return;
    final libraries = [
      for (final grant in current.grants.libraries)
        if (library == null || grant.libraryId != library.libraryId) grant,
      if (library != null) library,
    ];
    setState(() => saving = true);
    try {
      await (widget.saveGrants ?? setDeviceGrants)(
        SetDeviceGrantsRequest(
          fingerprint: widget.client.fingerprint,
          grants: DeviceGrantsChangeSet(
            value: DeviceGrantsInfo(
              control: control ?? current.grants.control,
              allConnection: current.grants.allConnection,
              allSync: current.grants.allSync,
              libraries: libraries,
              administer: current.grants.administer,
            ),
          ),
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
      builder: (context) => UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: SingleChildScrollView(
            padding: getScrollContainerPadding(context),
            child: SettingsBodyPadding(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  _sectionTitle(widget.client.alias),
                  if (current != null) ...[
                    SettingsBoxToggle(
                      title: s.controlPlayback,
                      subtitle: s.controlPlaybackDescription,
                      value: current.grants.control,
                      onChanged: saving
                          ? null
                          : (value) => _save(control: value),
                    ),
                    _sectionTitle(s.deviceAccessLibraries),
                    for (final library in current.libraries) ...[
                      _libraryAccess(s, library),
                    ],
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  // One choice per library sets both levels. A stored pair outside the three
  // choices is shown as Custom and is only replaced when the user picks one.
  Widget _libraryAccess(AppLocalizations s, DeviceLibraryAccess library) {
    final current = _LibraryAccess.of(library);
    return SettingsBoxComboBox<_LibraryAccess>(
      title: library.alias,
      subtitle: '',
      value: current,
      items: [
        for (final choice in _LibraryAccess.values)
          if (choice != _LibraryAccess.custom || current == choice)
            SettingsBoxComboBoxItem(value: choice, title: choice.title(s)),
      ],
      onChanged: saving
          ? null
          : (choice) {
              final grant = choice?.grant(library.libraryId);
              if (grant != null && choice != current) _save(library: grant);
            },
    );
  }

  Widget _sectionTitle(String title) => Padding(
    padding: const EdgeInsets.only(top: 8, bottom: 2, left: 6, right: 6),
    child: Text(title),
  );
}

/// The access a device page offers for a library. Each choice stands for one
/// connection level and one sync level; finer pairs come only from the API or
/// the grants file and show as [custom].
enum _LibraryAccess {
  notShared,
  readOnly,
  readWrite,
  custom;

  static _LibraryAccess of(DeviceLibraryAccess library) =>
      switch ((library.connection, library.sync)) {
        (DeviceConnectionLevel.none, DeviceSyncLevel.none) => notShared,
        (DeviceConnectionLevel.view, DeviceSyncLevel.pull) => readOnly,
        (DeviceConnectionLevel.viewEdit, DeviceSyncLevel.pullPush) => readWrite,
        _ => custom,
      };

  String title(AppLocalizations s) => switch (this) {
    notShared => s.deviceAccessNotShared,
    readOnly => s.readOnly,
    readWrite => s.deviceAccessReadWrite,
    custom => s.customDeviceAccess,
  };

  /// The grant that setting this choice writes, or null for [custom], which
  /// cannot be picked.
  DeviceLibraryGrant? grant(String libraryId) => switch (this) {
    notShared => DeviceLibraryGrant(
      libraryId: libraryId,
      connection: DeviceConnectionLevel.none,
      sync: DeviceSyncLevel.none,
    ),
    readOnly => DeviceLibraryGrant(
      libraryId: libraryId,
      connection: DeviceConnectionLevel.view,
      sync: DeviceSyncLevel.pull,
    ),
    readWrite => DeviceLibraryGrant(
      libraryId: libraryId,
      connection: DeviceConnectionLevel.viewEdit,
      sync: DeviceSyncLevel.pullPush,
    ),
    custom => null,
  };
}
