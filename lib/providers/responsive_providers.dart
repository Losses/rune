import 'package:fluent_ui/fluent_ui.dart';
import 'package:window_manager/window_manager.dart';
import 'dart:async';
import 'package:provider/provider.dart';

enum DeviceTpe { zune, phone, mobile, tablet, desktop, tv }

class ResponsiveBreakpoint {
  final double start;
  final double end;
  final DeviceTpe name;

  const ResponsiveBreakpoint(
      {required this.start, required this.end, required this.name});
}

class ResponsiveProvider extends ChangeNotifier with WindowListener {
  static const List<ResponsiveBreakpoint> breakpoints = [
    ResponsiveBreakpoint(start: 0, end: 320, name: DeviceTpe.zune),
    ResponsiveBreakpoint(start: 0, end: 480, name: DeviceTpe.phone),
    ResponsiveBreakpoint(start: 481, end: 650, name: DeviceTpe.mobile),
    ResponsiveBreakpoint(start: 651, end: 800, name: DeviceTpe.tablet),
    ResponsiveBreakpoint(start: 801, end: 1920, name: DeviceTpe.desktop),
    ResponsiveBreakpoint(start: 1921, end: double.infinity, name: DeviceTpe.tv),
  ];

  DeviceTpe _currentBreakpoint = DeviceTpe.desktop;
  DateTime? _lastUpdateTime;
  Timer? _throttleTimer;

  ResponsiveProvider() {
    windowManager.addListener(this);
    _updateBreakpoint();
  }

  DeviceTpe get currentBreakpoint => _currentBreakpoint;

  @override
  void onWindowResize() {
    final now = DateTime.now();
    if (_lastUpdateTime == null ||
        now.difference(_lastUpdateTime!) >= const Duration(milliseconds: 100)) {
      _updateBreakpoint();
      _lastUpdateTime = now;
    } else {
      _throttleTimer?.cancel();
      _throttleTimer = Timer(
          Duration(
              milliseconds:
                  100 - now.difference(_lastUpdateTime!).inMilliseconds),
          _updateBreakpoint);
    }
  }

  void _updateBreakpoint() async {
    final size = await windowManager.getSize();
    final width = size.width;
    final newBreakpoint = breakpoints
        .firstWhere(
          (bp) => width >= bp.start && width <= bp.end,
          orElse: () => breakpoints.last,
        )
        .name;

    if (newBreakpoint != _currentBreakpoint) {
      _currentBreakpoint = newBreakpoint;
      notifyListeners();
    }
  }

  bool smallerOrEqualTo(DeviceTpe breakpointName) {
    return _currentBreakpoint.index <= breakpointName.index;
  }

  bool largerOrEqualTo(DeviceTpe breakpointName) {
    return _currentBreakpoint.index >= breakpointName.index;
  }

  bool equalTo(DeviceTpe breakpointName) {
    return _currentBreakpoint == breakpointName;
  }

  @override
  void dispose() {
    windowManager.removeListener(this);
    _throttleTimer?.cancel();
    super.dispose();
  }
}

class SmallerOrEqualTo extends StatelessWidget {
  final DeviceTpe breakpoint;
  final Widget Function(BuildContext context, bool isTrue) builder;

  const SmallerOrEqualTo(
      {super.key, required this.breakpoint, required this.builder});

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, bool>(
      selector: (_, provider) => provider.smallerOrEqualTo(breakpoint),
      builder: (context, isTrue, child) => builder(context, isTrue),
    );
  }
}

class LargerOrEqualTo extends StatelessWidget {
  final DeviceTpe breakpoint;
  final Widget Function(BuildContext context, bool isTrue) builder;

  const LargerOrEqualTo(
      {super.key, required this.breakpoint, required this.builder});

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, bool>(
      selector: (_, provider) => provider.largerOrEqualTo(breakpoint),
      builder: (context, isTrue, child) => builder(context, isTrue),
    );
  }
}

class EqualTo extends StatelessWidget {
  final DeviceTpe breakpoint;
  final Widget Function(BuildContext context, bool isTrue) builder;

  const EqualTo({super.key, required this.breakpoint, required this.builder});

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, bool>(
      selector: (_, provider) => provider.equalTo(breakpoint),
      builder: (context, isTrue, child) => builder(context, isTrue),
    );
  }
}

class BreakpointBuilder extends StatelessWidget {
  final List<DeviceTpe> breakpoints;
  final Widget Function(BuildContext context, DeviceTpe activeBreakpoint)
      builder;

  const BreakpointBuilder({
    super.key,
    required this.breakpoints,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, DeviceTpe>(
      selector: (_, provider) => breakpoints.firstWhere(
        (bp) => provider.smallerOrEqualTo(bp),
        orElse: () => breakpoints.last,
      ),
      builder: (context, activeBreakpoint, child) =>
          builder(context, activeBreakpoint),
    );
  }
}
