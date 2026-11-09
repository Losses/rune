import 'package:fluent_ui/fluent_ui.dart';

import '../../bindings/bindings.dart';
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
                      SettingsBoxComboBox<DeviceConnectionLevel>(
                        title: library.alias,
                        subtitle: s.deviceConnection,
                        value: library.connection,
                        items: [
                          SettingsBoxComboBoxItem(
                            value: DeviceConnectionLevel.none,
                            title: s.deviceConnectionOff,
                          ),
                          SettingsBoxComboBoxItem(
                            value: DeviceConnectionLevel.view,
                            title: s.deviceConnectionView,
                          ),
                          SettingsBoxComboBoxItem(
                            value: DeviceConnectionLevel.viewEdit,
                            title: s.deviceConnectionViewEdit,
                          ),
                        ],
                        onChanged: saving
                            ? null
                            : (value) {
                                if (value != null &&
                                    value != library.connection) {
                                  _save(
                                    library: DeviceLibraryGrant(
                                      libraryId: library.libraryId,
                                      connection: value,
                                      sync: library.sync,
                                    ),
                                  );
                                }
                              },
                      ),
                      SettingsBoxComboBox<DeviceSyncLevel>(
                        title: library.alias,
                        subtitle: s.deviceSync,
                        value: library.sync,
                        items: [
                          SettingsBoxComboBoxItem(
                            value: DeviceSyncLevel.none,
                            title: s.deviceSyncOff,
                          ),
                          SettingsBoxComboBoxItem(
                            value: DeviceSyncLevel.pull,
                            title: s.deviceSyncPull,
                          ),
                          SettingsBoxComboBoxItem(
                            value: DeviceSyncLevel.pullPush,
                            title: s.deviceSyncPullPush,
                          ),
                        ],
                        onChanged: saving
                            ? null
                            : (value) {
                                if (value != null && value != library.sync) {
                                  _save(
                                    library: DeviceLibraryGrant(
                                      libraryId: library.libraryId,
                                      connection: library.connection,
                                      sync: value,
                                    ),
                                  );
                                }
                              },
                      ),
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

  Widget _sectionTitle(String title) => Padding(
    padding: const EdgeInsets.only(top: 8, bottom: 2, left: 6, right: 6),
    child: Text(title),
  );
}
