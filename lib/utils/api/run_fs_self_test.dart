import 'dart:async';
import 'dart:convert';

import '../../bindings/bindings.dart';

class FsSelfTestLayerResult {
  final String layer;
  final bool ok;
  final bool skipped;
  final int elapsedMs;
  final String detail;

  const FsSelfTestLayerResult({
    required this.layer,
    required this.ok,
    required this.skipped,
    required this.elapsedMs,
    required this.detail,
  });

  factory FsSelfTestLayerResult.fromJson(Map<String, dynamic> json) {
    return FsSelfTestLayerResult(
      layer: json['layer'] as String? ?? '',
      ok: json['ok'] as bool? ?? false,
      skipped: json['skipped'] as bool? ?? false,
      elapsedMs: (json['elapsed_ms'] as num?)?.toInt() ?? 0,
      detail: json['detail'] as String? ?? '',
    );
  }
}

class FsSelfTestResult {
  final bool success;
  final List<FsSelfTestLayerResult> layers;

  const FsSelfTestResult({required this.success, required this.layers});
}

Future<FsSelfTestResult> runFsSelfTest(
  String path, {
  void Function(FsSelfTestProgress progress)? onProgress,
}) async {
  final progressSubscription =
      FsSelfTestProgress.rustSignalStream.listen((rustSignal) {
    onProgress?.call(rustSignal.message);
  });

  try {
    RunFsSelfTestRequest(path: path).sendSignalToRust(); // GENERATED

    final rustSignal = await RunFsSelfTestResponse.rustSignalStream.first
        .timeout(const Duration(seconds: 120));
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

    return FsSelfTestResult(success: response.success, layers: layers);
  } finally {
    await progressSubscription.cancel();
  }
}
