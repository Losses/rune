import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/api/check_device_on_server.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../bindings/bindings.dart';

class ServerStatusDialog extends StatefulWidget {
  final List<String> hosts;
  final Function(CheckDeviceOnServerResponse?) close;

  const ServerStatusDialog({
    super.key,
    required this.hosts,
    required this.close,
  });

  @override
  ServerStatusDialogState createState() => ServerStatusDialogState();
}

class ServerStatusDialogState extends State<ServerStatusDialog> {
  StreamSubscription? _pollingSubscription;

  @override
  void initState() {
    super.initState();
    _startPolling();
  }

  @override
  void dispose() {
    _pollingSubscription?.cancel();
    super.dispose();
  }

  void _startPolling() {
    _pollingSubscription = Stream.periodic(const Duration(seconds: 2))
        .asyncMap((_) => _checkDeviceStatus())
        .listen(
      (response) {
        if (response.status != ClientStatus.pending) {
          widget.close(response);
        }
      },
      onError: (e) async {
        await _handlePollingError(e);
      },
    );
  }

  Future<CheckDeviceOnServerResponse> _checkDeviceStatus() async {
    return await checkDeviceOnServer(widget.hosts);
  }

  Future<void> _handlePollingError(Object error) async {
    _pollingSubscription?.cancel();
    widget.close(null);
    await showErrorDialog(
      context: context,
      title: S.of(context).unknownError,
      errorMessage: error.toString(),
    );
  }

  void _cancelPolling() {
    _pollingSubscription?.cancel();
    widget.close(null);
  }

  @override
  Widget build(BuildContext context) {
    return ContentDialog(
      constraints: const BoxConstraints(maxWidth: 400),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Center(child: ProgressRing()),
          const SizedBox(height: 20),
          Text(
            S.of(context).waitForApprove,
            textAlign: TextAlign.center,
          ),
        ],
      ),
      actions: [
        Button(
          onPressed: _cancelPolling,
          child: Text(S.of(context).cancel),
        ),
      ],
    );
  }
}
