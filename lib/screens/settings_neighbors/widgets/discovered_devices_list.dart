import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rinf/rinf.dart';

import '../../../messages/all.dart';
import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_tile_title.dart';

import '../utils/show_fingerprint_quiz_dialog.dart';

class DiscoveredDevicesList extends StatefulWidget {
  const DiscoveredDevicesList({super.key});

  @override
  State<DiscoveredDevicesList> createState() => _DiscoveredDevicesListState();
}

class _DiscoveredDevicesListState extends State<DiscoveredDevicesList> {
  String? _selectedFingerprint;
  List<DiscoveredDeviceMessage> _devices = [];
  Timer? _pollingTimer;
  StreamSubscription? _responseSubscription;
  late StreamSubscription<RustSignal<GetDiscoveredDeviceResponse>>
      _subscription;

  @override
  void initState() {
    super.initState();
    _startListening();
    _startDiscovery();
  }

  void _startListening() {
    _subscription =
        GetDiscoveredDeviceResponse.rustSignalStream.listen(_onData);
  }

  void _startDiscovery() {
    StartListeningRequest(alias: 'discovery').sendSignalToRust();

    _pollingTimer = Timer.periodic(const Duration(seconds: 2), (_) {
      _fetchDevices();
    });
    _fetchDevices();
  }

  Future<void> _fetchDevices() async {
    GetDiscoveredDeviceRequest().sendSignalToRust();
  }

  _onData(RustSignal<GetDiscoveredDeviceResponse> response) {
    setState(() {
      _devices = response.message.devices;
    });
  }

  @override
  void dispose() {
    _pollingTimer?.cancel();
    _responseSubscription?.cancel();

    final stopRequest = StopListeningRequest();
    stopRequest.sendSignalToRust();
    _subscription.cancel();

    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    if (_devices.isEmpty) {
      return Center(child: Text(s.noDevicesFound));
    }

    return ListView.builder(
      itemCount: _devices.length,
      itemBuilder: (context, index) {
        final device = _devices[index];
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
                const SizedBox(height: 12),
                Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  children: [
                    Button(
                      onPressed: () => _handlePairDevice(device),
                      child: Text(s.pair),
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
  }

  Widget _buildDetailItem(String label, String value) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4.0),
      child: RichText(
        text: TextSpan(
          style: const TextStyle(height: 1.4),
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

  void _handlePairDevice(DiscoveredDeviceMessage device) {
    if (device.ips.isNotEmpty) {
      showFingerprintQuizDialog(context, device.ips.first);
    }
  }
}

IconData deviceTypeToIcon(String deviceType) {
  switch (deviceType) {
    case "Mobile":
      return Symbols.smartphone;
    case "Desktop":
      return Symbols.computer;
    case "Web":
      return Symbols.public;
    case "Headless":
      return Symbols.psychology_alt;
    case "Server":
      return Symbols.host;
    default:
      return Symbols.help;
  }
}
