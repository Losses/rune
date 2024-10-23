import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'package:rune/messages/all.dart';

class CrashProvider with ChangeNotifier {
  String? report;

  late StreamSubscription<RustSignal<CrashResponse>> subscription;

  CrashProvider() {
    subscription = CrashResponse.rustSignalStream.listen(_updatePlaybackStatus);
  }

  @override
  void dispose() {
    super.dispose();
    subscription.cancel();
  }

  void _updatePlaybackStatus(RustSignal<CrashResponse> signal) {
    report = signal.message.detail;
    notifyListeners();
  }
}
