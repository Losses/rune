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
  late final ModalRouteEntry _entry;

  @override
  void initState() {
    super.initState();
    _entry = ModalRouteEntry(
      name: widget.name,
      arguments: widget.arguments,
      canPop: widget.canPop,
      pop: () => _handleClose(null, fromBack: true),
    );
    $history.pushModal(_entry);
  }

  /// The dialog's own entry leaves the history, wherever it is: a page may
  /// have been pushed over the dialog, or another dialog opened over it.
  @override
  void dispose() {
    // Barrier, Escape and direct Navigator pops can also dismiss this route.
    $history.remove(_entry);
    super.dispose();
  }

  void _handleClose(dynamic result, {bool fromBack = false}) {
    if (!mounted || !identical($history.current, _entry)) return;
    final route = ModalRoute.of(context);
    if (route == null || !route.isCurrent) return;
    if (fromBack && route.popDisposition == RoutePopDisposition.doNotPop) return;

    final canPop = _entry.canPop;
    if (canPop != null) {
      final (allowed, popResult) = canPop();
      if (!allowed) return;
      result = popResult;
    }

    Navigator.pop(context, result);
    // A route can consume a pop without leaving the Navigator.
    if (!route.isCurrent) $history.remove(_entry);
  }

  @override
  Widget build(BuildContext context) {
    // The route's safe area does not move a dialog clear of the on-screen
    // keyboard, so it rises with the keyboard the way Material's Dialog does.
    // The padding in the media data already excludes the keyboard, so the
    // dialog is not inset twice.
    return AnimatedPadding(
      padding: EdgeInsets.only(bottom: MediaQuery.viewInsetsOf(context).bottom),
      duration: const Duration(milliseconds: 100),
      curve: Curves.decelerate,
      child: MediaQuery.removeViewInsets(
        context: context,
        removeBottom: true,
        child: widget.builder(context, _handleClose),
      ),
    );
  }
}
