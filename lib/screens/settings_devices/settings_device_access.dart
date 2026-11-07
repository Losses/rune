import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../bindings/bindings.dart';
import '../../utils/api/device_grants.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/dialogs/information/error.dart';
import '../../utils/l10n.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/settings_page_padding.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/settings/settings_box_toggle.dart';
import '../../widgets/settings/settings_tile_title.dart';
import 'device_access.dart';

class SettingsDeviceAccessPage extends StatefulWidget {
  const SettingsDeviceAccessPage({super.key, required this.client});

  final ClientSummary client;

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
      final result = await getDeviceGrants(widget.client.fingerprint);
      if (mounted) setState(() => data = result);
    } catch (error) {
      if (mounted) _showError(error);
    }
  }

  Future<void> _save({bool? control, DeviceLibraryGrant? library}) async {
    setState(() => saving = true);
    try {
      await setDeviceGrants(
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
                  for (final library in current.libraries)
                    ListTile(
                      title: SettingsTileTitle(
                        icon: Symbols.library_music,
                        title: library.alias,
                        subtitle: '',
                        showActions: true,
                        actionsBuilder: (context) =>
                            ComboBox<DeviceAccessChoice>(
                              value: choiceForLevel(library.sync),
                              items: [
                                ComboBoxItem(
                                  value: DeviceAccessChoice.notShared,
                                  child: Text(s.deviceNotShared),
                                ),
                                ComboBoxItem(
                                  value: DeviceAccessChoice.read,
                                  child: Text(s.deviceReadOnly),
                                ),
                                ComboBoxItem(
                                  value: DeviceAccessChoice.readWrite,
                                  child: Text(s.deviceReadWrite),
                                ),
                              ],
                              onChanged: saving
                                  ? null
                                  : (value) {
                                      if (value != null &&
                                          value !=
                                              choiceForLevel(library.sync)) {
                                        _save(
                                          library: DeviceLibraryGrant(
                                            libraryId: library.libraryId,
                                            connection: switch (value) {
                                              DeviceAccessChoice.notShared =>
                                                DeviceConnectionLevel.none,
                                              DeviceAccessChoice.read =>
                                                DeviceConnectionLevel.view,
                                              DeviceAccessChoice.readWrite =>
                                                DeviceConnectionLevel.viewEdit,
                                            },
                                            sync: levelForChoice(value),
                                          ),
                                        );
                                      }
                                    },
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
