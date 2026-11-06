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
import '../../providers/library_path.dart';
import '../../utils/router/navigation.dart';
import '../settings_server/widgets/peer_libraries_dialog.dart';
import '../settings_neighbors/widgets/edit_device_information_setting_button.dart';
import '../settings_neighbors/widgets/search_remote_device_setting_button.dart';
import '../settings_neighbors/widgets/add_neighbor_manually_setting_button.dart';
import 'device_status.dart';
import 'device_access.dart';
import 'remote_destinations_list.dart';

import '../settings_server/utils/show_confirm_remove_device_dialog.dart';
import '../settings_server/utils/show_review_connection_dialog.dart';
import '../settings_server/widgets/server_control_setting_button.dart';
import '../settings_server/widgets/enable_broadcast_setting_button.dart';

class SettingsDevicesPage extends StatefulWidget {
  const SettingsDevicesPage({super.key});

  @override
  State<SettingsDevicesPage> createState() => _SettingsDevicesPageState();
}

class _SettingsDevicesPageState extends State<SettingsDevicesPage> {
  String selectedUserId = '';
  GetSyncSettingsResponse? syncSettings;
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
        context, listen: false,
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
    getSyncSettings()
        .then((value) {
          if (mounted) setState(() => syncSettings = value);
        })
        .catchError((Object error) {
          debugPrint('Could not load sync settings: $error');
        });
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
                  const EditDeviceInformationSettingButton(),
                  const EnableBroadcastSetting(),
                  const ServerControlSetting(),
                  if (syncSettings case final settings?)
                    SettingsBoxToggle(
                      title: S.of(context).allowPairedSync,
                      subtitle: S.of(context).allowPairedSyncDescription,
                      value: settings.listen,
                      onChanged: (value) async {
                        try {
                          await setSyncSettings(
                            value,
                            settings.missingHostFile,
                          );
                          final updated = await getSyncSettings();
                          if (mounted) setState(() => syncSettings = updated);
                        } catch (error) {
                          debugPrint('Could not update sync settings: $error');
                        }
                      },
                    ),
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
                  _sectionTitle(context, S.of(context).devicesThatCanConnect),
                  _buildUserListView(users, context),
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
    padding: const EdgeInsets.only(top: 24, bottom: 8),
    child: Text(title, style: FluentTheme.of(context).typography.subtitle),
  );

  Widget _buildUserListView(List<ClientSummary> clients, BuildContext context) {
    final broadcastProvider = Provider.of<BroadcastProvider>(context);

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
              subtitle: client.status == ClientStatus.approved
                  ? deviceAccessSummary(
                      grants[client.fingerprint], S.of(context),
                    )
                  : presentation.subtitle,
              showActions: selectedUserId == client.fingerprint,
              actionsBuilder: (context) => Wrap(
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  if (client.status == ClientStatus.approved) ...[
                    Button(
                      onPressed: () async {
                        await $push(
                          '/settings/device-access', arguments: client,
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
                  if (client.status == ClientStatus.approved &&
                      !(Provider.of<LibraryPathProvider>(
                            context,
                          ).currentPath?.startsWith('@RR|') ??
                          false)) ...[
                    Button(
                      onPressed: () => $showModal<void>(
                        context,
                        (context, close) => PeerLibrariesDialog(
                          client: client,
                          close: () => close(null),
                        ),
                      ),
                      child: Text(S.of(context).deviceLibraries),
                    ),
                    const SizedBox(width: 12),
                  ],
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
