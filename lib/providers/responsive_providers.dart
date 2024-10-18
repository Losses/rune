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
  final DeviceOrientation orientation;

  const ResponsiveBreakpoint({
    required this.start,
    required this.end,
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
  static const Map<DeviceType, ResponsiveBreakpoint> breakpoints = {
    DeviceType.dock: ResponsiveBreakpoint(
      start: 0,
      end: 120,
      orientation: DeviceOrientation.vertical,
    ),
    DeviceType.zune: ResponsiveBreakpoint(
      start: 121,
      end: 320,
      orientation: DeviceOrientation.vertical,
    ),
    DeviceType.phone: ResponsiveBreakpoint(
      start: 321,
      end: 480,
      orientation: DeviceOrientation.vertical,
    ),
    DeviceType.mobile: ResponsiveBreakpoint(
      start: 481,
      end: 650,
      orientation: DeviceOrientation.vertical,
    ),
    DeviceType.tablet: ResponsiveBreakpoint(
      start: 651,
      end: 800,
      orientation: DeviceOrientation.vertical,
    ),
    DeviceType.desktop: ResponsiveBreakpoint(
      start: 801,
      end: 1920,
      orientation: DeviceOrientation.vertical,
    ),
    DeviceType.tv: ResponsiveBreakpoint(
      start: 1921,
      end: double.infinity,
      orientation: DeviceOrientation.vertical,
    ),
    DeviceType.band: ResponsiveBreakpoint(
      start: 0,
      end: 44,
      orientation: DeviceOrientation.horizontal,
    ),
    DeviceType.fish: ResponsiveBreakpoint(
      start: 45,
      end: 120,
      orientation: DeviceOrientation.horizontal,
    ),
    DeviceType.car: ResponsiveBreakpoint(
      start: 121,
      end: 340,
      orientation: DeviceOrientation.horizontal,
    ),
    DeviceType.station: ResponsiveBreakpoint(
      start: 341,
      end: double.infinity,
      orientation: DeviceOrientation.horizontal,
    ),
  };

  DeviceType _currentVerticalBreakpoint = DeviceType.desktop;
  DeviceType _currentHorizontalBreakpoint = DeviceType.station;
  DeviceType currentBreakpoint = DeviceType.desktop;

  ResponsiveProvider(ScreenSizeProvider screenSizeProvider) {
    screenSizeProvider.addListener(_updateBreakpoints);
    _updateBreakpoints();
  }

  void _updateBreakpoints() {
    final size = ScreenSizeProvider().screenSize;
    final width = size.width;
    final height = size.height;

    _currentVerticalBreakpoint =
        _getBreakpoint(width, DeviceOrientation.vertical);
    _currentHorizontalBreakpoint =
        _getBreakpoint(height, DeviceOrientation.horizontal);

    final verticalPriority = devicePriority[_currentVerticalBreakpoint] ?? 0;
    final horizontalPriority =
        devicePriority[_currentHorizontalBreakpoint] ?? 0;
    currentBreakpoint = verticalPriority >= horizontalPriority
        ? _currentVerticalBreakpoint
        : _currentHorizontalBreakpoint;

    notifyListeners();
  }

  DeviceType _getBreakpoint(double size, DeviceOrientation orientation) {
    return breakpoints.entries
        .where((entry) => entry.value.orientation == orientation)
        .firstWhere(
          (entry) => size >= entry.value.start && size <= entry.value.end,
          orElse: () => breakpoints.entries.firstWhere((entry) =>
              entry.value.orientation == orientation &&
              entry.value.end == double.infinity),
        )
        .key;
  }

  DeviceType getActiveBreakpoint(List<DeviceType> breakpoints) {
    final verticalBreakpoints = breakpoints
        .where((bp) => getOrientation(bp) == DeviceOrientation.vertical)
        .toList();
    final horizontalBreakpoints = breakpoints
        .where((bp) => getOrientation(bp) == DeviceOrientation.horizontal)
        .toList();

    DeviceType? verticalActive = _getActiveForOrientation(
      verticalBreakpoints,
      _currentVerticalBreakpoint,
    );
    DeviceType? horizontalActive = _getActiveForOrientation(
      horizontalBreakpoints,
      _currentHorizontalBreakpoint,
    );

    if (verticalActive != null && horizontalActive != null) {
      return devicePriority[verticalActive]! >=
              devicePriority[horizontalActive]!
          ? verticalActive
          : horizontalActive;
    } else if (verticalActive != null) {
      return verticalActive;
    } else if (horizontalActive != null) {
      return horizontalActive;
    } else {
      return breakpoints.last;
    }
  }

  DeviceType? _getActiveForOrientation(
    List<DeviceType> x,
    DeviceType currentBreakpoint,
  ) {
    if (x.isEmpty) return null;

    // Filter the breakpoints to only include those in the list `x`
    final filteredBreakpoints =
        breakpoints.entries.where((entry) => x.contains(entry.key)).toList();

    // If no breakpoints match, return null
    if (filteredBreakpoints.isEmpty) return null;

    // Find the last breakpoint where a.end <= b.start
    DeviceType? result;
    for (final entry in filteredBreakpoints.reversed) {
      final a = breakpoints[currentBreakpoint]!;
      final b = entry.value;

      if (a.start > b.start) {
        return result;
      }

      result = entry.key;
    }

    // Return null if no suitable breakpoint is found
    return filteredBreakpoints[0].key;
  }

  static DeviceOrientation getOrientation(DeviceType deviceType) {
    return breakpoints[deviceType]!.orientation;
  }

  bool smallerOrEqualTo(DeviceType breakpointName,
      [bool strictOrientation = true]) {
    final orientation = getOrientation(breakpointName);

    if (!strictOrientation) {
      final activeOrientation = getOrientation(currentBreakpoint);
      if (activeOrientation != orientation) return false;
    }

    final selectedCompareTarget = orientation == DeviceOrientation.vertical
        ? _currentVerticalBreakpoint
        : _currentHorizontalBreakpoint;

    final a = breakpoints[selectedCompareTarget]!;
    final b = breakpoints[breakpointName]!;

    return a.start <= b.start;
  }

  bool largerOrEqualTo(DeviceType breakpointName,
      [bool strictOrientation = true]) {
    final orientation = getOrientation(breakpointName);

    if (!strictOrientation) {
      final activeOrientation = getOrientation(currentBreakpoint);
      if (activeOrientation != orientation) return false;
    }

    final selectedCompareTarget = orientation == DeviceOrientation.vertical
        ? _currentVerticalBreakpoint
        : _currentHorizontalBreakpoint;

    final a = breakpoints[selectedCompareTarget]!;
    final b = breakpoints[breakpointName]!;

    return a.end >= b.end;
  }

  bool equalTo(DeviceType breakpointName, [bool strictOrientation = true]) {
    final orientation = getOrientation(breakpointName);

    if (!strictOrientation) {
      final activeOrientation = getOrientation(currentBreakpoint);
      if (activeOrientation != orientation) return false;
    }

    final selectedCompareTarget = orientation == DeviceOrientation.vertical
        ? _currentVerticalBreakpoint
        : _currentHorizontalBreakpoint;

    final a = breakpoints[selectedCompareTarget];
    final b = breakpoints[breakpointName];

    return a == b;
  }
}

class SmallerOrEqualTo extends StatelessWidget {
  final DeviceType breakpoint;
  final Widget Function(BuildContext context, bool matches) builder;

  const SmallerOrEqualTo({
    super.key,
    required this.breakpoint,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, bool>(
      selector: (_, provider) {
        return provider.smallerOrEqualTo(breakpoint);
      },
      builder: (context, matches, child) => builder(context, matches),
    );
  }
}

class LargerOrEqualTo extends StatelessWidget {
  final DeviceType breakpoint;
  final Widget Function(BuildContext context, bool matches) builder;

  const LargerOrEqualTo({
    super.key,
    required this.breakpoint,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, bool>(
      selector: (_, provider) {
        return provider.largerOrEqualTo(breakpoint);
      },
      builder: (context, matches, child) => builder(context, matches),
    );
  }
}

class EqualTo extends StatelessWidget {
  final DeviceType breakpoint;
  final Widget Function(BuildContext context, bool matches) builder;

  const EqualTo({
    super.key,
    required this.breakpoint,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, bool>(
      selector: (_, provider) {
        return provider.equalTo(breakpoint);
      },
      builder: (context, matches, child) => builder(context, matches),
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
      selector: (_, provider) => provider.getActiveBreakpoint(breakpoints),
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
