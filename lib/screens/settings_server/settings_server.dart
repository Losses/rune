import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/settings/settings_tile_title.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../messages/all.dart';
import '../../providers/broadcast.dart';

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

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _initializeData();
    });
  }

  void _initializeData() {
    final broadcastProvider =
        Provider.of<BroadcastProvider>(context, listen: false);

    broadcastProvider.fetchUsers();

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

  Widget _buildUserListView(List<ClientSummary> users, BuildContext context) {
    return SizedBox(
      width: double.maxFinite,
      child: ListView.builder(
        shrinkWrap: true,
        physics: const NeverScrollableScrollPhysics(),
        itemCount: users.length,
        itemBuilder: (context, index) {
          final user = users[index];
          return ListTile.selectable(
            title: SettingsTileTitle(
              icon: Symbols.devices,
              badgeContent: Icon(
                _getStatusIcon(user.status),
                size: 12,
              ),
              title: user.alias,
              subtitle:
                  '${user.deviceModel} • ${_getStatusText(user.status, context)}',
              showActions: selectedUserId == user.fingerprint,
              actionsBuilder: (context) => Row(
                children: [
                  Button(
                    onPressed: () =>
                        showReviewConnectionDialog(context, user.fingerprint),
                    child: Text(S.of(context).review),
                  ),
                ],
              ),
            ),
            selected: selectedUserId == user.fingerprint,
            onSelectionChange: (selected) => setState(
              () => selectedUserId = selected ? user.fingerprint : '',
            ),
          );
        },
      ),
    );
  }

  IconData _getStatusIcon(ClientStatus status) {
    switch (status) {
      case ClientStatus.APPROVED:
        return Symbols.check_circle;
      case ClientStatus.PENDING:
        return Symbols.pending;
      case ClientStatus.BLOCKED:
        return Symbols.block;
    }

    return Symbols.help;
  }

  String _getStatusText(ClientStatus status, BuildContext context) {
    return switch (status) {
      ClientStatus.APPROVED => S.of(context).approvedStatus,
      ClientStatus.PENDING => S.of(context).pendingStatus,
      ClientStatus.BLOCKED => S.of(context).blockedStatus,
      ClientStatus() => S.of(context).unknownStatus,
    };
  }
}
