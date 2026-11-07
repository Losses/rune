import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../bindings/bindings.dart';
import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_tile_title.dart';

import '../utils/show_fingerprint_quiz_dialog.dart';

class DiscoveredDevicesList extends StatefulWidget {
  const DiscoveredDevicesList({
    super.key,
    required this.onPaired,
    required this.onAnswered,
  });

  final void Function() onPaired;
  final void Function(DiscoveredDeviceMessage, bool?) onAnswered;

  @override
  State<DiscoveredDevicesList> createState() => _DiscoveredDevicesListState();
}

class _DiscoveredDevicesListState extends State<DiscoveredDevicesList> {
  String? _selectedFingerprint;
  List<DiscoveredDeviceMessage> _devices = [];
  Timer? _pollingTimer;
  StreamSubscription? _responseSubscription;
  late StreamSubscription<RustSignalPack<GetDiscoveredDeviceResponse>>
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

  _onData(RustSignalPack<GetDiscoveredDeviceResponse> response) {
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

  void _handlePairDevice(DiscoveredDeviceMessage device) async {
    if (device.ips.isNotEmpty) {
      widget.onPaired();
      final result = await showFingerprintQuizDialog(context, device.ips.first);

      widget.onAnswered(device, result);
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
