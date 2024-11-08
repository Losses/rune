import 'package:fluent_ui/fluent_ui.dart';

import 'history.dart';
import 'modal_route_entry.dart';

class ModalRouteWrapper extends StatefulWidget {
  final String name;
  final Object? arguments;
  final (bool, dynamic) Function()? canPop;
  final Widget Function(
      BuildContext context, void Function(dynamic result) close) builder;

  const ModalRouteWrapper({
    super.key,
    required this.name,
    this.arguments,
    this.canPop,
    required this.builder,
  });

  @override
  State<ModalRouteWrapper> createState() => _ModalRouteWrapperState();
}

class _ModalRouteWrapperState extends State<ModalRouteWrapper> {
  @override
  void initState() {
    super.initState();
    $history.pushModal(
      ModalRouteEntry(
        name: widget.name,
        arguments: widget.arguments,
        canPop: widget.canPop,
        pop: () {
          _handleClose(null);
        },
      ),
    );
  }

  @override
  void dispose() {
    if ($history.isCurrentModal) {
      $history.pop();
    }
    super.dispose();
  }

  void _handleClose(dynamic result) {
    final current = $history.current;
    if (current is! ModalRouteEntry) return;

    bool canPop = true;
    if (current.canPop != null) {
      final (confirm, popResult) = current.canPop!();
      canPop = confirm;
      if (canPop) {
        result = popResult;
      }
    }

    if (canPop && mounted) {
      Navigator.pop(context, result);
    }
  }

  @override
  Widget build(BuildContext context) {
    return widget.builder(context, _handleClose);
  }
}
