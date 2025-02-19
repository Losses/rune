import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../../providers/discovery.dart';
import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_tile_title.dart';
import '../utils/show_fingerprint_quiz_dialog.dart';

class DiscoveredDevicesList extends StatefulWidget {
  const DiscoveredDevicesList({super.key});

  @override
  State<DiscoveredDevicesList> createState() => _DiscoveredDevicesListState();
}

class _DiscoveredDevicesListState extends State<DiscoveredDevicesList> {
  late final DiscoveryProvider _provider;

  String? _selectedFingerprint;

  @override
  void initState() {
    super.initState();
    _provider = context.read<DiscoveryProvider>();
    _provider.startListening();
  }

  @override
  void dispose() {
    _provider.stopListening();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return Consumer<DiscoveryProvider>(
      builder: (context, provider, _) {
        if (provider.error != null) {
          return Center(
            child: Text(s.error(provider.error!)),
          );
        }

        if (provider.devices.isEmpty) {
          return Center(
            child: Text(s.noDevicesFound),
          );
        }

        return ListView.builder(
          itemCount: provider.devices.length,
          itemBuilder: (context, index) {
            final device = provider.devices.values.elementAt(index);
            final isSelected = _selectedFingerprint == device.fingerprint;

            return ListTile.selectable(
              title: SettingsTileTitle(
                icon: deviceTypeToIcon(device.deviceType),
                title: device.alias,
                subtitle: device.deviceModel,
                showActions: isSelected,
                actionsBuilder: (context) => Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    _buildDetailItem(s.fingerprint, device.fingerprint),
                    _buildDetailItem(
                      s.ipAddresses,
                      device.ips.join(', '),
                    ),
                    SizedBox(height: 12),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.end,
                      children: [
                        Button(
                          onPressed: () => _handlePairDevice(device),
                          child: Text(S.of(context).pair),
                        ),
                      ],
                    )
                  ],
                ),
              ),
              selected: isSelected,
              onSelectionChange: (v) =>
                  setState(() => _selectedFingerprint = device.fingerprint),
            );
          },
        );
      },
    );
  }

  Widget _buildDetailItem(String label, String value) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4.0),
      child: RichText(
        text: TextSpan(
          style: TextStyle(height: 1.4),
          children: [
            TextSpan(
              text: '$label: ',
              style: const TextStyle(fontWeight: FontWeight.bold),
            ),
            TextSpan(text: value),
          ],
        ),
      ),
    );
  }

  void _handlePairDevice(DiscoveredDevice device) {
    showFingerprintQuizDialog(context, device.ips[0]);
  }
}

IconData deviceTypeToIcon(String deviceType) {
  if (deviceType == "Mobile") {
    return Symbols.smartphone;
  }

  if (deviceType == "Desktop") {
    return Symbols.computer;
  }

  if (deviceType == "Web") {
    return Symbols.public;
  }

  if (deviceType == "Headless") {
    return Symbols.psychology_alt;
  }

  if (deviceType == "Server") {
    return Symbols.host;
  }

  return Symbols.help;
}
