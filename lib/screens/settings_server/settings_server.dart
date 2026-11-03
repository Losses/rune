import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../constants/configurations.dart';
import '../../constants/settings_manager.dart';
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
import 'widgets/peer_libraries_dialog.dart';

import 'utils/show_confirm_remove_device_dialog.dart';
import 'utils/show_review_connection_dialog.dart';
import 'widgets/server_control_setting_button.dart';
import 'widgets/enable_broadcast_setting_button.dart';

class SettingsServerPage extends StatefulWidget {
  const SettingsServerPage({super.key});

  @override
  State<SettingsServerPage> createState() => _SettingsServerPageState();
}

class _SettingsServerPageState extends State<SettingsServerPage> {
  late Timer _refreshTimer;
  String selectedUserId = '';
  GetSyncSettingsResponse? syncSettings;
  bool offlineCleanupDefault = true;

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

    _refreshTimer = Timer.periodic(const Duration(seconds: 3), (_) {
      broadcastProvider.fetchUsers();
    });
  }

  @override
  void dispose() {
    _refreshTimer.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final broadcastProvider = Provider.of<BroadcastProvider>(context);
    final users = broadcastProvider.users;

    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: SingleChildScrollView(
            padding: getScrollContainerPadding(context),
            child: SettingsBodyPadding(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
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
                  const ServerControlSetting(),
                  const EnableBroadcastSetting(),
                  const SizedBox(height: 8),
                  _buildUserListView(users, context),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

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
          return ListTile.selectable(
            title: SettingsTileTitle(
              icon: Symbols.devices,
              badgeContent: Icon(
                _getStatusIcon(client.status),
                size: 12,
                color: Colors.white,
              ),
              badgeColor: _getStatusColor(client.status),
              title: client.alias,
              subtitle:
                  '${client.deviceModel} • ${_getStatusText(client.status, context)}',
              showActions: selectedUserId == client.fingerprint,
              actionsBuilder: (context) => Row(
                children: [
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

  String _getStatusText(ClientStatus status, BuildContext context) {
    return switch (status) {
      ClientStatus.approved => S.of(context).approvedStatus,
      ClientStatus.pending => S.of(context).pendingStatus,
      ClientStatus.blocked => S.of(context).blockedStatus,
    };
  }
}
