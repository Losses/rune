import 'request_failure.dart';
import 'dart:async';
import 'dart:convert';

import '../../bindings/bindings.dart';
import '../settings_manager.dart';

class FsSelfTestLayerResult {
  final String layer;
  final FsSelfTestLayerOutcome outcome;
  final int elapsedMs;
  final String detail;

  const FsSelfTestLayerResult({
    required this.layer,
    required this.outcome,
    required this.elapsedMs,
    required this.detail,
  });

  factory FsSelfTestLayerResult.fromJson(Map<String, dynamic> json) {
    return FsSelfTestLayerResult(
      layer: json['layer'] as String? ?? '',
      outcome: switch ((json['ok'] as bool? ?? false, json['skipped'] as bool? ?? false)) {
        (true, false) => FsSelfTestLayerOutcome.passed,
        (false, false) => FsSelfTestLayerOutcome.failed,
        (_, true) => FsSelfTestLayerOutcome.skipped,
      },
      elapsedMs: (json['elapsed_ms'] as num?)?.toInt() ?? 0,
      detail: json['detail'] as String? ?? '',
    );
  }
}

class FsSelfTestResult {
  final bool success;
  final List<FsSelfTestLayerResult> layers;
  final WalSelfTestResult walProbe;

  const FsSelfTestResult({
    required this.success,
    required this.layers,
    required this.walProbe,
  });
}

class WalSelfTestResult {
  final bool supported;
  final String? detail;

  const WalSelfTestResult({required this.supported, this.detail});
}

Future<FsSelfTestResult> runFsSelfTest(
  String path, {
  void Function(FsSelfTestProgress progress)? onProgress,
}) async {
  final dbPath = await getSettingsPath();
  final progressSubscription = FsSelfTestProgress.rustSignalStream.listen((
    rustSignal,
  ) {
    onProgress?.call(rustSignal.message);
  });

  try {
    final rustSignal = await awaitResponse(
      'RunFsSelfTestRequest',
      RunFsSelfTestResponse.rustSignalStream,
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (callId) => RunFsSelfTestRequest(
        path: path,
        dbPath: dbPath,
        callId: callId,
      ).sendSignalToRust(),
      timeout: const Duration(seconds: 120),
    );
    final response = rustSignal.message;

    List<FsSelfTestLayerResult> layers = [];
    try {
      final decoded = jsonDecode(response.reportJson);
      if (decoded is List) {
        layers = decoded
            .whereType<Map<String, dynamic>>()
            .map(FsSelfTestLayerResult.fromJson)
            .toList();
      }
    } on FormatException {
      // Keep the raw progress the UI already received.
    }

    return FsSelfTestResult(
      success: response.success,
      layers: layers,
      walProbe: WalSelfTestResult(
        supported: response.walProbe.supported,
        detail: response.walProbe.detail,
      ),
    );
  } finally {
    await progressSubscription.cancel();
  }
}
