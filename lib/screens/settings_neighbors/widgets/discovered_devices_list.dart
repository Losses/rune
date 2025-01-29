import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../../../providers/discovery.dart';
import '../../../utils/l10n.dart';

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
            final isExpanded = _selectedFingerprint == device.fingerprint;

            return ListTile(
              title: _DeviceTitle(
                alias: device.alias,
                isExpanded: isExpanded,
                device: device,
                onPairPressed: () => _handlePairDevice(device),
              ),
              subtitle: isExpanded
                  ? Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        _buildDetailItem(s.model, device.deviceModel),
                        _buildDetailItem(s.type, device.deviceType),
                        _buildDetailItem(s.fingerprint, device.fingerprint),
                        _buildDetailItem(
                          s.lastSeen,
                          '${device.lastSeen.toLocal()}',
                        ),
                        _buildDetailItem(
                          s.ipAddresses,
                          device.ips.join(', '),
                        ),
                      ],
                    )
                  : null,
              onTap: () => setState(() {
                _selectedFingerprint = isExpanded ? null : device.fingerprint;
              }),
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
          style: Theme.of(context).textTheme.bodyMedium,
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
    // PairDeviceRequest(
    //   fingerprint: device.fingerprint,
    //   alias: device.alias,
    // ).sendSignalToRust();

    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(S.of(context).pairingWith(device.alias)),
      ),
    );
  }
}

class _DeviceTitle extends StatelessWidget {
  final String alias;
  final bool isExpanded;
  final DiscoveredDevice device;
  final VoidCallback onPairPressed;

  const _DeviceTitle({
    required this.alias,
    required this.isExpanded,
    required this.device,
    required this.onPairPressed,
  });

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Expanded(
          child: Text(
            alias,
            style: Theme.of(context).textTheme.titleMedium,
          ),
        ),
        if (isExpanded)
          Row(
            children: [
              FilledButton(
                onPressed: onPairPressed,
                child: Text(S.of(context).pair),
              ),
              const SizedBox(width: 12),
            ],
          ),
      ],
    );
  }
}
