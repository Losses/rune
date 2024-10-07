import 'dart:async';
import 'dart:math';
import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/scheduler.dart';

import '../../screens/settings_about/widgets/cover_grid.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../widgets/playback_controller/playback_placeholder.dart';

const flipInterval = 2;

class SettingsMixPage extends StatefulWidget {
  const SettingsMixPage({super.key});

  @override
  State<SettingsMixPage> createState() => _SettingsMixPageState();
}

class _SettingsMixPageState extends State<SettingsMixPage> {
  @override
  Widget build(BuildContext context) {
    return const Column(children: [
      NavigationBarPlaceholder(),
      Padding(
        padding: EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: SizedBox(
          width: 120,
          height: 120,
          child: FlipCoverGrid(
            paths: [
              'assets/demo_cover.webp',
              'assets/demo_cover.webp',
              'assets/demo_cover.webp',
              'assets/demo_cover.webp'
            ],
            size: 120,
          ),
        ),
      ),
      PlaybackPlaceholder(),
    ]);
  }
}

class FlipCoverGrid extends StatefulWidget {
  final List<String> paths;
  final int speed;
  final int size;

  const FlipCoverGrid({
    super.key,
    required this.paths,
    required this.size,
    this.speed = 500,
  });

  @override
  FlipCoverGridState createState() => FlipCoverGridState();
}

class FlipCoverGridState extends State<FlipCoverGrid>
    with SingleTickerProviderStateMixin {
  late int _gridCount;
  late DateTime _lastFlipTime;
  late List<String> _frontPaths;
  late List<String> _backPaths;
  late List<bool> _isFront;
  late List<bool> _isFlipping;
  late List<DateTime?> _flipStartTimes;
  late List<double> _rotates;
  final Random _random = Random();
  final Map<String, ui.Image> _imageCache = {};
  late Ticker _ticker;
  bool _isExecuting = false;

  @override
  void initState() {
    super.initState();
    _initializeGrid();
    _updateCache();
    _ticker = Ticker(_onTick)..start();
  }

  void _initializeGrid() {
    _lastFlipTime = DateTime.now();
    _gridCount = _determineGridSize();
    _frontPaths = List.from(widget.paths);
    _backPaths = List.from(widget.paths);
    _frontPaths.shuffle();
    _backPaths.shuffle();
    _isFront = List.filled(_gridCount * _gridCount, false);
    _isFlipping = List.filled(_gridCount * _gridCount, false);
    _flipStartTimes = List.filled(_gridCount * _gridCount, null);
    _rotates = List.filled(_gridCount * _gridCount, 0.0);
  }

  int _determineGridSize() {
    if (widget.paths.length < 4) return 1;
    if (widget.paths.length < 9) return 2;
    return 3;
  }

  void _onTick(Duration elapsed) {
    if (!_isExecuting) {
      _check();
    }

    _updateRotations();
  }

  void _check() {
    if (DateTime.now().difference(_lastFlipTime).inSeconds >= flipInterval) {
      _isExecuting = true;
      _lastFlipTime = DateTime.now();
      _prepareFlip();
      _updateCache().then((_) {
        _isExecuting = false;
      });
    }
  }

  Future<void> _updateCache() async {
    final Set<String> currentPaths = {
      ..._frontPaths.take(_gridCount * _gridCount),
      ..._backPaths.take(_gridCount * _gridCount),
    };

    _imageCache.keys
        .where((key) => !currentPaths.contains(key))
        .toList()
        .forEach(_imageCache.remove);

    for (final path in currentPaths) {
      if (!_imageCache.containsKey(path)) {
        await _loadAndCacheImage(path);
      }
    }
  }

  Future<void> _loadAndCacheImage(String path) async {
    final ByteData data = await rootBundle.load(path);
    final Uint8List bytes = data.buffer.asUint8List();
    final int size = (widget.size / _gridCount).ceil();

    final ui.Codec codec = await ui.instantiateImageCodec(
      bytes,
      targetWidth: size,
      targetHeight: size,
    );
    final ui.FrameInfo fi = await codec.getNextFrame();
    _imageCache[path] = fi.image;
  }

  void _stageFlipGridData(int index) {
    const int maxAttempts = 10;
    int attempts = 0;
    String newPath;

    do {
      newPath = widget.paths[_random.nextInt(widget.paths.length)];
      attempts++;
    } while ((_frontPaths.contains(newPath) ||
            _backPaths.contains(newPath) ||
            _frontPaths[index] == newPath ||
            _backPaths[index] == newPath) &&
        attempts < maxAttempts);

    if (attempts < maxAttempts) {
      if (_isFront[index] == true) {
        _backPaths[index] = newPath;
      } else {
        _frontPaths[index] = newPath;
      }
    }
  }

  void _prepareFlip() {
    for (int k = 0; k < _gridCount * _gridCount; k++) {
      if (_random.nextDouble() > 0.64) {
        _isFlipping[k] = true;
        _flipStartTimes[k] = DateTime.now();
        _stageFlipGridData(k);
      } else {
        _isFlipping[k] = false;
        _flipStartTimes[k] = null;
      }
    }
  }

  void _updateRotations() {
    bool needsUpdate = false;
    for (int k = 0; k < _gridCount * _gridCount; k++) {
      if (_isFlipping[k] && _flipStartTimes[k] != null) {
        final elapsedTime =
            DateTime.now().difference(_flipStartTimes[k]!).inMilliseconds;
        _rotates[k] = (elapsedTime / widget.speed) * pi;
        if (_rotates[k] >= pi) {
          _rotates[k] = 0;
          _isFlipping[k] = false;
          _flipStartTimes[k] = null;
          _isFront[k] = !_isFront[k];
        }
        needsUpdate = true;
      } else {
        _rotates[k] = 0;
      }
    }
    if (needsUpdate) {
      setState(() {});
    }
  }

  @override
  Widget build(BuildContext context) {
    final image = _imageCache[_frontPaths[0]];

    if (image == null) {
      return Container();
    } else {
      return CoverGrid(
        image: image,
        rotates: _rotates,
        gridCount: _gridCount,
      );
    }
  }

  @override
  void dispose() {
    _ticker.dispose();
    _imageCache.clear();
    super.dispose();
  }
}
