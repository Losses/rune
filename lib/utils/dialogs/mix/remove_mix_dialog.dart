import '../../api/request_failure.dart';
import '../../../bindings/bindings.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/responsive_dialog_actions.dart';
import '../../../utils/l10n.dart';

import '../../api/remove_mix.dart';
import '../../router/navigation.dart';

import '../remove_dialog_on_band.dart';

Future<bool?> showRemoveMixDialog(BuildContext context, int mixId) async {
  final result = await $showModal<bool>(
    context,
    (context, $close) => RemoveMixDialog(mixId: mixId, $close: $close),
    dismissWithEsc: true,
    barrierDismissible: true,
  );

  return result;
}

class RemoveMixDialog extends StatefulWidget {
  final int mixId;
  final void Function(bool?) $close;

  const RemoveMixDialog({super.key, required this.mixId, required this.$close});

  @override
  State<RemoveMixDialog> createState() => _RemoveMixDialogState();
}

class _RemoveMixDialogState extends State<RemoveMixDialog> {
  bool isLoading = false;
  RequestFailure? failure;

  void _onConfirm() async {
    setState(() {
      isLoading = true;
    });
    try {
      await removeMix(widget.mixId);
    } on RequestFailure catch (error) {
      if (error.kind != RequestErrorKind.notFound) {
        if (mounted) {
          setState(() {
            isLoading = false;
            failure = error;
          });
        }
        return;
      }
    }

    if (!mounted) return;

    widget.$close(true);
  }

  @override
  Widget build(BuildContext context) {
    return RemoveDialogOnBand(
      $close: widget.$close,
      onConfirm: _onConfirm,
      child: ContentDialog(
        title: Column(
          children: [SizedBox(height: 8), Text(S.of(context).removeMixTitle)],
        ),
        content: Text(
          failure == null
              ? S.of(context).removeMixSubtitle
              : requestFailureMessage(S.of(context), failure!),
        ),
        actions: [
          ResponsiveDialogActions(
            Button(
              onPressed: isLoading ? null : _onConfirm,
              child: Text(S.of(context).delete),
            ),
            FilledButton(
              onPressed: isLoading ? null : () => widget.$close(false),
              child: Text(S.of(context).cancel),
            ),
          ),
        ],
      ),
    );
  }
}
