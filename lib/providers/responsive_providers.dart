import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

enum DeviceOrientation {
  vertical,
  horizontal,
}

enum DeviceType {
  dock(
    priority: 5,
    orientation: DeviceOrientation.vertical,
    start: 0,
    end: 120,
  ),
  zune(
    priority: 4,
    orientation: DeviceOrientation.vertical,
    start: 121,
    end: 340,
  ),
  phone(
    priority: 1,
    orientation: DeviceOrientation.vertical,
    start: 341,
    end: 480,
  ),
  mobile(
    priority: 1,
    orientation: DeviceOrientation.vertical,
    start: 481,
    end: 650,
  ),
  tablet(
    priority: 1,
    orientation: DeviceOrientation.vertical,
    start: 651,
    end: 800,
  ),
  desktop(
    priority: 1,
    orientation: DeviceOrientation.vertical,
    start: 801,
    end: 1920,
  ),
  tv(
    priority: 1,
    orientation: DeviceOrientation.vertical,
    start: 1921,
    end: double.infinity,
  ),
  band(
    priority: 6,
    orientation: DeviceOrientation.horizontal,
    start: 0,
    end: 148,
  ),
  belt(
    priority: 5,
    orientation: DeviceOrientation.horizontal,
    start: 149,
    end: 240,
  ),
  fish(
    priority: 3,
    orientation: DeviceOrientation.horizontal,
    start: 241,
    end: 300,
  ),
  car(
    priority: 2,
    orientation: DeviceOrientation.horizontal,
    start: 301,
    end: 340,
  ),
  station(
    priority: 0,
    orientation: DeviceOrientation.horizontal,
    start: 341,
    end: double.infinity,
  );

  final int priority;
  final DeviceOrientation orientation;

  final double start;
  final double end;

  const DeviceType({
    required this.priority,
    required this.orientation,
    required this.start,
    required this.end,
  });

  static DeviceType _determineDeviceType(
      {double? size, DeviceOrientation? orientation}) {
    assert(size != null || orientation != null,
        'At least one of size or orientation must be provided');

    return DeviceType.values
        .where((type) => orientation == null || type.orientation == orientation)
        .firstWhere(
          (type) => size == null || (size >= type.start && size <= type.end),
          orElse: () => DeviceType.values.firstWhere(
            (type) =>
                (orientation == null || type.orientation == orientation) &&
                type.end == double.infinity,
          ),
        );
  }

  DeviceType? _getActiveForOrientation(List<DeviceType> filteredDeviceTypes) {
    if (filteredDeviceTypes.isEmpty) return null;

    DeviceType? result;

    for (final entry in filteredDeviceTypes.reversed) {
      final a = this;
      final b = entry;

      if (a.start > b.start) {
        return result;
      }

      result = entry;
    }

    return filteredDeviceTypes.first;
  }
}

class ScreenSizeProvider extends ChangeNotifier with WidgetsBindingObserver {
  Size _screenSize = Size.zero;
  ScreenSizeProvider() {
    WidgetsBinding.instance.addObserver(this);
    _updateScreenSize();
  }

  Size get screenSize => _screenSize;

  @override
  void didChangeMetrics() {
    super.didChangeMetrics();
    _updateScreenSize();
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
    super.dispose();
  }
}

class ResponsiveProvider extends ChangeNotifier {
  DeviceType _currentVerticalDeviceType = DeviceType.desktop;
  DeviceType _currentHorizontalDeviceType = DeviceType.station;
  DeviceType currentDeviceType = DeviceType.desktop;

  ResponsiveProvider(ScreenSizeProvider screenSizeProvider) {
    screenSizeProvider.addListener(_updateDeviceTypes);
    _updateDeviceTypes();
  }

  void _updateDeviceTypes() {
    final size = ScreenSizeProvider().screenSize;
    final width = size.width;
    final height = size.height;

    final oldV = _currentVerticalDeviceType;
    final oldH = _currentHorizontalDeviceType;
    final oldA = currentDeviceType;

    final newV = DeviceType._determineDeviceType(
      size: width,
      orientation: DeviceOrientation.vertical,
    );
    final newH = DeviceType._determineDeviceType(
      size: height,
      orientation: DeviceOrientation.horizontal,
    );

    final verticalPriority = newV.priority;
    final horizontalPriority = newH.priority;
    final newA = verticalPriority >= horizontalPriority ? newV : newH;

    _currentVerticalDeviceType = newV;
    _currentHorizontalDeviceType = newH;

    currentDeviceType = newA;

    if (oldA != newA || oldV != newV || oldH != newH) {
      notifyListeners();
    }
  }

  DeviceType getActiveDeviceType(List<DeviceType> deviceTypes) {
    final verticalDeviceTypes = deviceTypes
        .where((bp) => bp.orientation == DeviceOrientation.vertical)
        .toList();
    final horizontalDeviceTypes = deviceTypes
        .where((bp) => bp.orientation == DeviceOrientation.horizontal)
        .toList();

    DeviceType? verticalActive =
        _currentVerticalDeviceType._getActiveForOrientation(
      verticalDeviceTypes,
    );
    DeviceType? horizontalActive =
        _currentHorizontalDeviceType._getActiveForOrientation(
      horizontalDeviceTypes,
    );

    if (verticalActive != null && horizontalActive != null) {
      return verticalActive.priority >= horizontalActive.priority
          ? verticalActive
          : horizontalActive;
    } else if (verticalActive != null) {
      return verticalActive;
    } else if (horizontalActive != null) {
      return horizontalActive;
    } else {
      return deviceTypes.last;
    }
  }

  bool _compareDeviceType(
      DeviceType deviceType, bool Function(DeviceType, DeviceType) comparison,
      [bool strictOrientation = true]) {
    if (!strictOrientation &&
        currentDeviceType.orientation != deviceType.orientation) {
      return false;
    }

    final selectedCompareTarget =
        deviceType.orientation == DeviceOrientation.vertical
            ? _currentVerticalDeviceType
            : _currentHorizontalDeviceType;

    return comparison(selectedCompareTarget, deviceType);
  }

  bool smallerOrEqualTo(DeviceType deviceType, [bool autoOrientation = true]) =>
      _compareDeviceType(
          deviceType, (a, b) => a.start <= b.start, autoOrientation);

  bool largerOrEqualTo(DeviceType deviceType,
          [bool strictOrientation = true]) =>
      _compareDeviceType(
          deviceType, (a, b) => a.end >= b.end, strictOrientation);

  bool equalTo(DeviceType deviceType, [bool strictOrientation = true]) =>
      _compareDeviceType(deviceType, (a, b) => a == b, strictOrientation);
}

class SmallerOrEqualTo extends StatelessWidget {
  final DeviceType deviceType;
  final Widget Function(BuildContext context, bool matches) builder;

  const SmallerOrEqualTo({
    super.key,
    required this.deviceType,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, bool>(
      selector: (_, provider) {
        return provider.smallerOrEqualTo(deviceType);
      },
      builder: (context, matches, child) => builder(context, matches),
    );
  }
}

class LargerOrEqualTo extends StatelessWidget {
  final DeviceType deviceType;
  final Widget Function(BuildContext context, bool matches) builder;

  const LargerOrEqualTo({
    super.key,
    required this.deviceType,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, bool>(
      selector: (_, provider) {
        return provider.largerOrEqualTo(deviceType);
      },
      builder: (context, matches, child) => builder(context, matches),
    );
  }
}

class EqualTo extends StatelessWidget {
  final DeviceType deviceType;
  final Widget Function(BuildContext context, bool matches) builder;

  const EqualTo({
    super.key,
    required this.deviceType,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, bool>(
      selector: (_, provider) {
        return provider.equalTo(deviceType);
      },
      builder: (context, matches, child) => builder(context, matches),
    );
  }
}

class DeviceTypeBuilder extends StatelessWidget {
  final List<DeviceType> deviceType;
  final Widget Function(BuildContext context, DeviceType activeDeviceType)
      builder;

  const DeviceTypeBuilder({
    super.key,
    required this.deviceType,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ResponsiveProvider, DeviceType>(
      selector: (_, provider) => provider.getActiveDeviceType(deviceType),
      builder: (context, activeDeviceType, child) =>
          builder(context, activeDeviceType),
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

bool smallerThanWatch(Size size) {
  return size.width <= 64 || size.height <= 64;
}

class SmallerOrEqualThanWatch extends StatelessWidget {
  final Widget Function(BuildContext context, bool isWatch) builder;

  const SmallerOrEqualThanWatch({
    super.key,
    required this.builder,
  });

  @override
  Widget build(BuildContext context) {
    return Selector<ScreenSizeProvider, bool>(
      selector: (_, screenSizeProvider) {
        final width = screenSizeProvider.screenSize.width;
        final height = screenSizeProvider.screenSize.height;
        return width <= 64 || height <= 64;
      },
      builder: (context, isWatch, child) => builder(context, isWatch),
    );
  }
}
