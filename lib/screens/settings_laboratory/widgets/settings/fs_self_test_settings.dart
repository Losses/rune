import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../../bindings/bindings.dart';
import '../../../../utils/api/run_fs_self_test.dart';
import '../../../../utils/get_dir_path.dart';
import '../../../../utils/router/navigation.dart';

import '../settings_card.dart';

class FsSelfTestSettings extends StatelessWidget {
  const FsSelfTestSettings({super.key});

  @override
  Widget build(BuildContext context) {
    return SettingsCard(
      title: "Filesystem Self-Test",
      description:
          "Pick a directory and diagnose the Dart / Rust / SAF file I/O layers on it. "
          "Use this on a TF card to verify file access end to end.",
      content: Button(
        onPressed: () async {
          final path = await getDirPath();

          if (path == null) return;
          if (!context.mounted) return;

          await $showModal<void>(
            context,
            (context, $close) => FsSelfTestDialog(path: path, $close: $close),
            barrierDismissible: true,
            dismissWithEsc: true,
          );
        },
        child: const Text('Run self-test'),
      ),
    );
  }
}

class FsSelfTestDialog extends StatefulWidget {
  const FsSelfTestDialog({
    super.key,
    required this.path,
    required this.$close,
  });

  final String path;
  final void Function(void) $close;

  @override
  State<FsSelfTestDialog> createState() => _FsSelfTestDialogState();
}

class _FsSelfTestDialogState extends State<FsSelfTestDialog> {
  final List<FsSelfTestLayerResult> _layers = [];
  bool _finished = false;
  bool _success = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _run();
  }

  Future<void> _run() async {
    try {
      final result = await runFsSelfTest(
        widget.path,
        onProgress: (FsSelfTestProgress progress) {
          if (!mounted) return;
          setState(() {
            _layers.add(
              FsSelfTestLayerResult(
                layer: progress.layer,
                ok: progress.ok,
                skipped: progress.skipped,
                elapsedMs: progress.elapsedMs.toInt(),
                detail: progress.detail,
              ),
            );
          });
        },
      );

      if (!mounted) return;
      setState(() {
        _finished = true;
        _success = result.success;
        if (result.layers.isNotEmpty) {
          _layers
            ..clear()
            ..addAll(result.layers);
        }
      });
    } on TimeoutException {
      if (!mounted) return;
      setState(() {
        _finished = true;
        _error =
            'No response from the Rust backend within 120 seconds. '
            'The Rust side may have failed to initialize; check logcat for details.';
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _finished = true;
        _error = '$e';
      });
    }
  }

  Widget _buildStatusIcon(FsSelfTestLayerResult layer) {
    if (layer.skipped) {
      return Icon(Symbols.block, color: Colors.grey, size: 18);
    }
    if (layer.ok) {
      return Icon(Symbols.check_circle, color: Colors.green, size: 18);
    }
    return Icon(Symbols.error, color: Colors.red, size: 18);
  }

  Widget _buildSummary(BuildContext context) {
    if (!_finished) {
      return const Row(
        children: [
          ProgressRing(),
          SizedBox(width: 12),
          Text('Running self-test...'),
        ],
      );
    }

    if (_error != null) {
      return Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(Symbols.error, color: Colors.red, size: 18),
          const SizedBox(width: 8),
          Expanded(child: Text(_error!)),
        ],
      );
    }

    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Icon(
          _success ? Symbols.check_circle : Symbols.error,
          color: _success ? Colors.green : Colors.red,
          size: 18,
        ),
        const SizedBox(width: 8),
        Expanded(
          child: Text(
            _success
                ? 'All filesystem layers passed.'
                : 'Some filesystem layers failed. Expand the failed layers below for details.',
          ),
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    return ContentDialog(
      title: const Column(
        children: [
          SizedBox(height: 8),
          Text('Filesystem self-test'),
        ],
      ),
      constraints: const BoxConstraints(maxHeight: 480, maxWidth: 560),
      content: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _buildSummary(context),
          const SizedBox(height: 12),
          Expanded(
            child: ListView.builder(
              itemCount: _layers.length,
              itemBuilder: (context, index) {
                final layer = _layers[index];
                return Padding(
                  padding: const EdgeInsets.symmetric(vertical: 2),
                  child: Expander(
                    leading: _buildStatusIcon(layer),
                    header: Text(
                      '${layer.layer} · ${layer.elapsedMs} ms',
                      style: TextStyle(
                        color: layer.skipped ? Colors.grey : null,
                      ),
                    ),
                    content: SelectableText(
                      layer.detail.isEmpty ? '(no detail)' : layer.detail,
                      style: const TextStyle(height: 1.4),
                    ),
                  ),
                );
              },
            ),
          ),
        ],
      ),
      actions: [
        Button(
          onPressed: () => widget.$close(null),
          child: const Text('Close'),
        ),
      ],
    );
  }
}
