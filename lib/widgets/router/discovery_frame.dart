import 'package:flutter/material.dart';

import '../../screens/settings_server/utils/show_review_connection_dialog.dart';

import '../../bindings/bindings.dart';

class DiscoveryFrame extends StatefulWidget {
  const DiscoveryFrame(
    this.child, {
    super.key,
  });

  final Widget child;

  @override
  DiscoveryFrameState createState() => DiscoveryFrameState();
}

final _lastHandledMap = <String, DateTime>{};

class DiscoveryFrameState extends State<DiscoveryFrame> {
  @override
  void initState() {
    super.initState();

    IncommingClientPermissionNotification.rustSignalStream.listen((data) {
      if (!mounted) return;

      final client = data.message.user;
      final fingerprint = client.fingerprint;

      if (fingerprint.isEmpty) {
        showReviewConnectionDialog(context, client);
        return;
      }

      final now = DateTime.now();
      final lastHandledTime = _lastHandledMap[fingerprint];

      if (lastHandledTime == null ||
          now.difference(lastHandledTime).inSeconds >= 5) {
        _lastHandledMap[fingerprint] = now;
        showReviewConnectionDialog(context, client);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    return widget.child;
  }
}
