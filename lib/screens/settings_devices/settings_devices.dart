import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../constants/configurations.dart';
import '../../constants/settings_manager.dart';
import '../../utils/api/device_grants.dart';
import '../../utils/api/get_sync_settings.dart';
import '../../utils/api/set_sync_settings.dart';
import '../../widgets/settings/settings_box_toggle.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/settings/settings_tile_title.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../bindings/bindings.dart';
import '../../providers/broadcast.dart';
import '../../providers/controllers.dart';
import '../../utils/router/navigation.dart';
import '../settings_neighbors/widgets/edit_device_information_setting_button.dart';
import '../settings_neighbors/widgets/search_remote_device_setting_button.dart';
import '../settings_neighbors/widgets/add_neighbor_manually_setting_button.dart';
import 'device_status.dart';
import 'device_access.dart';
import 'remote_destinations_list.dart';

import '../settings_server/utils/show_confirm_remove_device_dialog.dart';
import '../settings_server/utils/show_review_connection_dialog.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/dialogs/information/error.dart';

class SettingsDevicesPage extends StatefulWidget {
  const SettingsDevicesPage({
    super.key,
    this.loadSyncSettings = getSyncSettings,
    this.saveSyncSettings = setSyncSettings,
  });

  final Future<GetSyncSettingsResponse> Function() loadSyncSettings;
  final Future<void> Function(SyncSettingsChange change) saveSyncSettings;

  @override
  State<SettingsDevicesPage> createState() => _SettingsDevicesPageState();
}

class _SettingsDevicesPageState extends State<SettingsDevicesPage> {
  String selectedUserId = '';
  GetSyncSettingsResponse? syncSettings;
  bool savingSyncSettings = false;
  bool offlineCleanupDefault = true;
  final Map<String, DeviceGrantsInfo> grants = {};
  BroadcastProvider? broadcast;
  bool loadingGrants = false;
  bool refreshGrantsAgain = false;

  Future<void> _refreshGrants() async {
    if (loadingGrants) {
      refreshGrantsAgain = true;
      return;
    }
    loadingGrants = true;
    do {
      refreshGrantsAgain = false;
      final clients = Provider.of<BroadcastProvider>(
        context,
        listen: false,
      ).users;
      for (final client in clients.where(
        (c) => c.status == ClientStatus.approved,
      )) {
        try {
          final result = await getDeviceGrants(client.fingerprint);
          if (!mounted) return;
          setState(() => grants[client.fingerprint] = result.grants);
        } catch (error) {
          if (!mounted) return;
          setState(() => grants.remove(client.fingerprint));
          debugPrint('Could not load device settings: $error');
        }
      }
    } while (refreshGrantsAgain && mounted);
    loadingGrants = false;
  }

  void _onClientsChanged() {
    if (mounted) _refreshGrants();
  }

  @override
  void dispose() {
    broadcast?.removeListener(_onClientsChanged);
    super.dispose();
  }

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _initializeData();
    });
  }

  void _initializeData() {
    final broadcastProvider = Provider.of<BroadcastProvider>(
      context,
      listen: false,
    );

    broadcastProvider.fetchUsers();
    broadcast = broadcastProvider;
    broadcastProvider.addListener(_onClientsChanged);
    _refreshGrants();
    $settingsManager.getValue<bool>(kOfflineCleanupDefaultKey).then((value) {
      if (mounted) setState(() => offlineCleanupDefault = value ?? true);
    });
    widget
        .loadSyncSettings()
        .then((value) {
          if (mounted) setState(() => syncSettings = value);
        })
        .catchError((Object error) {
          if (mounted) {
            showErrorDialog(
              context: context,
              title: S.of(context).mediaServer,
              errorMessage: requestErrorText(S.of(context), error),
            );
          }
        });
  }

  /// Saves [change] and shows it applied to the settings shown here. Only the
  /// changed fields are sent, so a setting changed elsewhere since this page
  /// loaded is not written back. A failed save throws, which leaves the
  /// switches at the values still in effect. The switches stay disabled until
  /// this finishes, so that edits are made one at a time.
  Future<void> _saveSyncSettings(
    String title,
    SyncSettingsChange change,
  ) async {
    setState(() => savingSyncSettings = true);
    try {
      try {
        await widget.saveSyncSettings(change);
        final shown = syncSettings;
        if (mounted && shown != null) {
          setState(() => syncSettings = change.applyTo(shown));
        }
      } finally {
        if (mounted) setState(() => savingSyncSettings = false);
      }
    } catch (error) {
      if (!mounted) return;
      await showErrorDialog(
        context: context,
        title: title,
        errorMessage: requestErrorText(S.of(context), error),
      );
    }
  }

  List<Widget> _mediaServerSettings(
    BuildContext context,
    GetSyncSettingsResponse settings,
  ) {
    final s = S.of(context);
    Widget permission(
      String title,
      String subtitle,
      bool value,
      SyncSettingsChange Function(bool value) change,
    ) => SettingsBoxToggle(
      title: title,
      subtitle: subtitle,
      value: value,
      onChanged: settings.mediaServer && !savingSyncSettings
          ? (value) => _saveSyncSettings(title, change(value))
          : null,
    );

    return [
      SettingsBoxToggle(
        title: s.mediaServer,
        subtitle: s.mediaServerSubtitle,
        value: settings.mediaServer,
        onChanged: savingSyncSettings
            ? null
            : (value) => _saveSyncSettings(
                s.mediaServer,
                SyncSettingsChange(mediaServer: value),
              ),
      ),
      permission(
        s.enableBroadcast,
        s.enableBroadcastSubtitle,
        settings.broadcast,
        (value) => SyncSettingsChange(broadcast: value),
      ),
      permission(
        s.allowRemoteControl,
        s.allowRemoteControlDescription,
        settings.acceptRemoteControl,
        (value) => SyncSettingsChange(acceptRemoteControl: value),
      ),
      permission(
        s.allowPairedSync,
        s.allowPairedSyncDescription,
        settings.acceptSync,
        (value) => SyncSettingsChange(acceptSync: value),
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final broadcastProvider = Provider.of<BroadcastProvider>(context);
    final users = broadcastProvider.users;

    return PageContentFrame(
      builder: (context) => UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: SingleChildScrollView(
            padding: getScrollContainerPadding(context),
            child: SettingsBodyPadding(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  _sectionTitle(context, S.of(context).thisDevice),
                  if (syncSettings case final settings?) ...[
                    ..._mediaServerSettings(context, settings),
                    // Not a Media Server permission: it is about this device
                    // reaching out, which needs no server.
                    SettingsBoxToggle(
                      title: S.of(context).autoDeviceSync,
                      subtitle: S.of(context).autoDeviceSyncDescription,
                      value: settings.autoDeviceSync,
                      onChanged: savingSyncSettings
                          ? null
                          : (value) => _saveSyncSettings(
                              S.of(context).autoDeviceSync,
                              SyncSettingsChange(autoDeviceSync: value),
                            ),
                    ),
                  ],
                  const EditDeviceInformationSettingButton(),
                  SettingsBoxToggle(
                    title: S.of(context).offlineCleanupDefault,
                    subtitle: S.of(context).offlineCleanupDefaultDescription,
                    value: offlineCleanupDefault,
                    onChanged: (value) {
                      setState(() => offlineCleanupDefault = value);
                      $settingsManager.setValue(
                        kOfflineCleanupDefaultKey,
                        value,
                      );
                    },
                  ),
                  if (users.isNotEmpty) ...[
                    _sectionTitle(context, S.of(context).devicesThatCanConnect),
                    _buildUserListView(users, context),
                  ],
                  _sectionTitle(context, S.of(context).otherDevices),
                  const SearchRemoteDeviceSettingButton(
                    tryClose: true,
                    navigateIfFailed: true,
                  ),
                  const AddNeighborManuallySettingButton(
                    tryClose: true,
                    navigateIfFailed: true,
                  ),
                  const RemoteDestinationsList(),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  Widget _sectionTitle(BuildContext context, String title) => Padding(
    padding: const EdgeInsets.only(top: 8, bottom: 2, left: 6, right: 6),
    child: Text(title),
  );

  Widget _buildUserListView(List<ClientSummary> clients, BuildContext context) {
    final broadcastProvider = Provider.of<BroadcastProvider>(context);
    final controllersProvider = Provider.of<ControllersProvider>(context);

    return SizedBox(
      width: double.maxFinite,
      child: ListView.builder(
        shrinkWrap: true,
        physics: const NeverScrollableScrollPhysics(),
        itemCount: clients.length,
        itemBuilder: (context, index) {
          final client = clients[index];
          final presentation = deviceStatusPresentation(
            client.status,
            S.of(context),
          );
          final status = deviceRowSubtitle(
            status: client.status,
            grants: grants[client.fingerprint],
            isControlling: controllersProvider.isControlling(
              client.fingerprint,
            ),
            strings: S.of(context),
          );
          return ListTile.selectable(
            title: SettingsTileTitle(
              icon: Symbols.devices,
              badgeContent: presentation.showBadge
                  ? Icon(
                      _getStatusIcon(client.status),
                      size: 12,
                      color: Colors.white,
                    )
                  : null,
              badgeColor: _getStatusColor(client.status),
              title: client.alias,
              subtitle: status,
              showActions: selectedUserId == client.fingerprint,
              actionsBuilder: (context) => Row(
                children: [
                  DisconnectControllerAction(fingerprint: client.fingerprint),
                  if (client.status == ClientStatus.approved) ...[
                    Button(
                      onPressed: () async {
                        await $push(
                          '/settings/device-access',
                          arguments: client,
                        );
                        if (mounted) _refreshGrants();
                      },
                      child: Text(S.of(context).customizeDevice),
                    ),
                    const SizedBox(width: 12),
                  ],
                  Button(
                    onPressed: () async {
                      await showReviewConnectionDialog(context, client);
                      broadcastProvider.fetchUsers();
                    },
                    child: Text(S.of(context).review),
                  ),
                  const SizedBox(width: 12),
                  Button(
                    onPressed: () async {
                      await showConfirmRemoveDeviceDialog(context, client);
                      broadcastProvider.fetchUsers();
                    },
                    child: Text(S.of(context).remove),
                  ),
                ],
              ),
            ),
            selected: selectedUserId == client.fingerprint,
            onSelectionChange: (selected) => setState(
              () => selectedUserId = selected ? client.fingerprint : '',
            ),
          );
        },
      ),
    );
  }

  IconData _getStatusIcon(ClientStatus status) {
    switch (status) {
      case ClientStatus.approved:
        return Symbols.check_circle;
      case ClientStatus.pending:
        return Symbols.pending;
      case ClientStatus.blocked:
        return Symbols.block;
    }
  }

  Color _getStatusColor(ClientStatus status) {
    switch (status) {
      case ClientStatus.approved:
        return Colors.green;
      case ClientStatus.pending:
        return Colors.yellow;
      case ClientStatus.blocked:
        return Colors.red;
    }
  }
}

class DisconnectControllerAction extends StatelessWidget {
  const DisconnectControllerAction({required this.fingerprint, super.key});

  final String fingerprint;

  @override
  Widget build(BuildContext context) {
    final controllersProvider = Provider.of<ControllersProvider>(context);
    if (!controllersProvider.isControlling(fingerprint)) {
      return const SizedBox.shrink();
    }

    return Row(
      children: [
        Button(
          onPressed: () async {
            try {
              await controllersProvider.disconnect(fingerprint);
            } catch (error) {
              if (!context.mounted) return;
              showErrorDialog(
                context: context,
                title: S.of(context).disconnect,
                errorMessage: requestErrorText(S.of(context), error),
              );
            }
          },
          child: Text(S.of(context).disconnect),
        ),
        const SizedBox(width: 12),
      ],
    );
  }
}
