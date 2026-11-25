import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../bindings/bindings.dart';
import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_tile_title.dart';

class DiscoveredDevicesList extends StatefulWidget {
  const DiscoveredDevicesList({
    super.key,
    required this.onSelected,
    this.initialDevices,
  });

  /// Called when the user chooses to pair with a device that has an address.
  final void Function(DiscoveredDeviceMessage) onSelected;

  /// Devices to show without discovering any, for a preview.
  final List<DiscoveredDeviceMessage>? initialDevices;

  @override
  State<DiscoveredDevicesList> createState() => _DiscoveredDevicesListState();
}

class _DiscoveredDevicesListState extends State<DiscoveredDevicesList> {
  String? _selectedFingerprint;
  List<DiscoveredDeviceMessage> _devices = [];
  Timer? _pollingTimer;
  StreamSubscription? _responseSubscription;
  StreamSubscription<RustSignalPack<GetDiscoveredDeviceResponse>>?
  _subscription;

  @override
  void initState() {
    super.initState();
    final initial = widget.initialDevices;
    if (initial != null) {
      _devices = initial;
      return;
    }
    _startListening();
    _startDiscovery();
  }

  void _startListening() {
    _subscription = GetDiscoveredDeviceResponse.rustSignalStream.listen(
      _onData,
    );
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

    if (widget.initialDevices == null) {
      final stopRequest = StopListeningRequest();
      stopRequest.sendSignalToRust();
      _subscription?.cancel();
    }

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
                      onPressed: device.addresses.isEmpty
                          ? null
                          : () => widget.onSelected(device),
                      child: Text(s.pair),
                    ),
                  ],
                ),
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
}

IconData deviceTypeToIcon(DeviceTypeMessage deviceType) {
  switch (deviceType) {
    case DeviceTypeMessage.mobile:
      return Symbols.smartphone;
    case DeviceTypeMessage.desktop:
      return Symbols.computer;
    case DeviceTypeMessage.web:
      return Symbols.public;
    case DeviceTypeMessage.headless:
      return Symbols.psychology_alt;
    case DeviceTypeMessage.server:
      return Symbols.host;
    case DeviceTypeMessage.unknown:
      return Symbols.help;
  }
}
