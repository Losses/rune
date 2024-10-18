import 'dart:async';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

enum DeviceOrientation {
  vertical,
  horizontal,
}

enum DeviceType {
  // Vertical
  dock,
  zune,
  phone,
  mobile,
  tablet,
  desktop,
  tv,
  // Horizontal
  band,
  fish,
  car,
  station,
}

const Map<DeviceType, int> devicePriority = {
  DeviceType.band: 4,
  DeviceType.fish: 4,
  DeviceType.car: 2,
  DeviceType.station: 0,
  DeviceType.dock: 3,
  DeviceType.zune: 3,
  DeviceType.phone: 1,
  DeviceType.mobile: 1,
  DeviceType.tablet: 1,
  DeviceType.desktop: 1,
  DeviceType.tv: 1,
};

class ResponsiveBreakpoint {
  final double start;
  final double end;
  final DeviceType name;
  final DeviceOrientation orientation;

  const ResponsiveBreakpoint({
    required this.start,
    required this.end,
    required this.name,
    required this.orientation,
  });
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
    final size = firstView.physicalSize / firstView.devicePixelRatio;
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
    ResponsiveBreakpoint(
      start: 0,
      end: 120,
      name: DeviceType.dock,
      orientation: DeviceOrientation.vertical,
    ),
    ResponsiveBreakpoint(
      start: 121,
      end: 320,
      name: DeviceType.zune,
      orientation: DeviceOrientation.vertical,
    ),
    ResponsiveBreakpoint(
      start: 321,
      end: 480,
      name: DeviceType.phone,
      orientation: DeviceOrientation.vertical,
    ),
    ResponsiveBreakpoint(
      start: 481,
      end: 650,
      name: DeviceType.mobile,
      orientation: DeviceOrientation.vertical,
    ),
    ResponsiveBreakpoint(
      start: 651,
      end: 800,
      name: DeviceType.tablet,
      orientation: DeviceOrientation.vertical,
    ),
    ResponsiveBreakpoint(
      start: 801,
      end: 1920,
      name: DeviceType.desktop,
      orientation: DeviceOrientation.vertical,
    ),
    ResponsiveBreakpoint(
      start: 1921,
      end: double.infinity,
      name: DeviceType.tv,
      orientation: DeviceOrientation.vertical,
    ),
    ResponsiveBreakpoint(
      start: 0,
      end: 120,
      name: DeviceType.band,
      orientation: DeviceOrientation.horizontal,
    ),
    ResponsiveBreakpoint(
      start: 121,
      end: 320,
      name: DeviceType.fish,
      orientation: DeviceOrientation.horizontal,
    ),
    ResponsiveBreakpoint(
      start: 321,
      end: 650,
      name: DeviceType.car,
      orientation: DeviceOrientation.horizontal,
    ),
    ResponsiveBreakpoint(
      start: 651,
      end: double.infinity,
      name: DeviceType.station,
      orientation: DeviceOrientation.horizontal,
    ),
  ];

  DeviceType _currentVerticalBreakpoint = DeviceType.desktop;
  DeviceType _currentHorizontalBreakpoint = DeviceType.station;

  ResponsiveProvider(ScreenSizeProvider screenSizeProvider) {
    screenSizeProvider.addListener(_updateBreakpoints);
    _updateBreakpoints();
  }

  DeviceType get currentBreakpoint {
    final verticalPriority = devicePriority[_currentVerticalBreakpoint] ?? 0;
    final horizontalPriority =
        devicePriority[_currentHorizontalBreakpoint] ?? 0;
    return verticalPriority >= horizontalPriority
        ? _currentVerticalBreakpoint
        : _currentHorizontalBreakpoint;
  }

  void _updateBreakpoints() {
    final size = ScreenSizeProvider().screenSize;
    final width = size.width;
    final height = size.height;

    _currentVerticalBreakpoint =
        _getBreakpoint(width, DeviceOrientation.vertical);
    _currentHorizontalBreakpoint =
        _getBreakpoint(height, DeviceOrientation.horizontal);

    notifyListeners();
  }

  DeviceType _getBreakpoint(double size, DeviceOrientation orientation) {
    return breakpoints
        .where((bp) => bp.orientation == orientation)
        .firstWhere(
          (bp) => size >= bp.start && size <= bp.end,
          orElse: () => breakpoints.firstWhere((bp) =>
              bp.orientation == orientation && bp.end == double.infinity),
        )
        .name;
  }

  bool smallerOrEqualTo(DeviceType breakpointName) {
    final orientation = _getOrientation(breakpointName);
    final currentBreakpoint = orientation == DeviceOrientation.vertical
        ? _currentVerticalBreakpoint
        : _currentHorizontalBreakpoint;
    return devicePriority[currentBreakpoint]! <=
        devicePriority[breakpointName]!;
  }

  bool largerOrEqualTo(DeviceType breakpointName) {
    final orientation = _getOrientation(breakpointName);
    final currentBreakpoint = orientation == DeviceOrientation.vertical
        ? _currentVerticalBreakpoint
        : _currentHorizontalBreakpoint;
    return devicePriority[currentBreakpoint]! >=
        devicePriority[breakpointName]!;
  }

  bool equalTo(DeviceType breakpointName) {
    final orientation = _getOrientation(breakpointName);
    final currentBreakpoint = orientation == DeviceOrientation.vertical
        ? _currentVerticalBreakpoint
        : _currentHorizontalBreakpoint;
    return currentBreakpoint == breakpointName;
  }

  DeviceOrientation _getOrientation(DeviceType deviceType) {
    return breakpoints.firstWhere((bp) => bp.name == deviceType).orientation;
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
  final double maxSize;
  final DeviceOrientation orientation;
  final Widget Function(BuildContext context, bool isSmaller) builder;

  const SmallerOrEqualToScreenSize({
    super.key,
    required this.maxSize,
    this.orientation = DeviceOrientation.vertical,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ScreenSizeProvider, bool>(
      selector: (_, screenSizeProvider) {
        final size = orientation == DeviceOrientation.vertical
            ? screenSizeProvider.screenSize.width
            : screenSizeProvider.screenSize.height;
        return size <= maxSize;
      },
      builder: (context, isSmaller, child) => builder(context, isSmaller),
    );
  }
}

class GreaterOrEqualToScreenSize extends StatelessWidget {
  final double minSize;
  final DeviceOrientation orientation;
  final Widget Function(BuildContext context, bool isGreater) builder;

  const GreaterOrEqualToScreenSize({
    super.key,
    required this.minSize,
    this.orientation = DeviceOrientation.vertical,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ScreenSizeProvider, bool>(
      selector: (_, screenSizeProvider) {
        final size = orientation == DeviceOrientation.vertical
            ? screenSizeProvider.screenSize.width
            : screenSizeProvider.screenSize.height;
        return size >= minSize;
      },
      builder: (context, isGreater, child) => builder(context, isGreater),
    );
  }
}
