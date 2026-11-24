import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../bindings/bindings.dart';

class CrashProvider with ChangeNotifier {
  String? report;

  late StreamSubscription<RustSignalPack<CrashResponse>> subscription;

  CrashProvider() {
    subscription = CrashResponse.rustSignalStream.listen(_updatePlaybackStatus);
  }

  @override
  void dispose() {
    super.dispose();
    subscription.cancel();
  }

  void _updatePlaybackStatus(RustSignalPack<CrashResponse> signal) {
    report = signal.message.detail;
    notifyListeners();
  }
}
