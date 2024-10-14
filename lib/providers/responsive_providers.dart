import 'dart:async';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

enum DeviceType { band, zune, phone, mobile, tablet, desktop, tv }

class ResponsiveBreakpoint {
  final double start;
  final double end;
  final DeviceType name;

  const ResponsiveBreakpoint(
      {required this.start, required this.end, required this.name});
}

class ScreenSizeProvider extends ChangeNotifier with WidgetsBindingObserver {
  Size _screenSize = Size.zero;
  DateTime? _lastUpdateTime;
  Timer? _throttleTimer;

  ScreenSizeProvider() {
    WidgetsBinding.instance.addObserver(this);
    _updateScreenSize();
  }

  Size get screenSize => _screenSize;

  @override
  void didChangeMetrics() {
    super.didChangeMetrics();

    final now = DateTime.now();
    if (_lastUpdateTime == null ||
        now.difference(_lastUpdateTime!) >= const Duration(milliseconds: 100)) {
      _updateScreenSize();
      _lastUpdateTime = now;
    } else {
      _throttleTimer?.cancel();
      _throttleTimer = Timer(
        Duration(
            milliseconds:
                100 - now.difference(_lastUpdateTime!).inMilliseconds),
        _updateScreenSize,
      );
    }
  }

  void _updateScreenSize() {
    final firstView = WidgetsBinding.instance.platformDispatcher.views.first;
    final size = firstView.physicalSize;
    if (size != _screenSize) {
      _screenSize = size;
      notifyListeners();
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _throttleTimer?.cancel();
    super.dispose();
  }
}

class ResponsiveProvider extends ChangeNotifier {
  static const List<ResponsiveBreakpoint> breakpoints = [
    ResponsiveBreakpoint(start: 0, end: 240, name: DeviceType.band),
    ResponsiveBreakpoint(start: 241, end: 640, name: DeviceType.zune),
    ResponsiveBreakpoint(start: 641, end: 960, name: DeviceType.phone),
    ResponsiveBreakpoint(start: 961, end: 1300, name: DeviceType.mobile),
    ResponsiveBreakpoint(start: 1301, end: 1600, name: DeviceType.tablet),
    ResponsiveBreakpoint(start: 1601, end: 3840, name: DeviceType.desktop),
    ResponsiveBreakpoint(
        start: 3841, end: double.infinity, name: DeviceType.tv),
  ];

  DeviceType _currentBreakpoint = DeviceType.desktop;

  ResponsiveProvider(ScreenSizeProvider screenSizeProvider) {
    screenSizeProvider.addListener(_updateBreakpoint);
    _updateBreakpoint();
  }

  DeviceType get currentBreakpoint => _currentBreakpoint;

  void _updateBreakpoint() {
    final width = ScreenSizeProvider().screenSize.width;
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

  bool smallerOrEqualTo(DeviceType breakpointName) {
    return _currentBreakpoint.index <= breakpointName.index;
  }

  bool largerOrEqualTo(DeviceType breakpointName) {
    return _currentBreakpoint.index >= breakpointName.index;
  }

  bool equalTo(DeviceType breakpointName) {
    return _currentBreakpoint == breakpointName;
  }
}

class SmallerOrEqualTo extends StatelessWidget {
  final DeviceType breakpoint;
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
  final DeviceType breakpoint;
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
  final DeviceType breakpoint;
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
  final List<DeviceType> breakpoints;
  final Widget Function(BuildContext context, DeviceType activeBreakpoint)
      builder;

  const BreakpointBuilder({
    super.key,
    required this.breakpoints,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, DeviceType>(
      selector: (_, provider) => breakpoints.firstWhere(
        (bp) => provider.smallerOrEqualTo(bp),
        orElse: () => breakpoints.last,
      ),
      builder: (context, activeBreakpoint, child) =>
          builder(context, activeBreakpoint),
    );
  }
}

class SmallerOrEqualToScreenSize extends StatelessWidget {
  final double maxWidth;
  final Widget Function(BuildContext context, bool isSmaller) builder;

  const SmallerOrEqualToScreenSize({
    super.key,
    required this.maxWidth,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ScreenSizeProvider, bool>(
      selector: (_, screenSizeProvider) =>
          screenSizeProvider.screenSize.width <= maxWidth,
      builder: (context, isSmaller, child) => builder(context, isSmaller),
    );
  }
}

class GreaterOrEqualToScreenSize extends StatelessWidget {
  final double minWidth;
  final Widget Function(BuildContext context, bool isSmaller) builder;

  const GreaterOrEqualToScreenSize({
    super.key,
    required this.minWidth,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ScreenSizeProvider, bool>(
      selector: (_, screenSizeProvider) =>
          screenSizeProvider.screenSize.width >= minWidth,
      builder: (context, isSmaller, child) => builder(context, isSmaller),
    );
  }
}
