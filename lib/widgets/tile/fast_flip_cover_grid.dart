import 'dart:math';
import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/scheduler.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'utils/hash13.dart';
import 'utils/image_proxy.dart';
import 'utils/flip_grid_painter.dart';
import 'constants/image_memory_manager.dart';

const flipInterval = 8;

class FastFlipCoverGrid extends StatefulWidget {
  final List<String> paths;
  final int speed;
  final int size;
  final String name;
  final List<Color> colors;

  const FastFlipCoverGrid({
    super.key,
    required this.paths,
    required this.size,
    required this.name,
    required this.colors,
    this.speed = 500,
  });

  @override
  FastFlipCoverGridState createState() => FastFlipCoverGridState();
}

class FastFlipCoverGridState extends State<FastFlipCoverGrid>
    with SingleTickerProviderStateMixin {
  late int _gridCount;
  late DateTime _lastFlipTime;
  late List<String> _frontPaths;
  late List<String> _backPaths;
  late List<Color> _frontColors;
  late List<Color> _backColors;
  late List<bool> _isFront;
  late List<bool> _isFlipping;
  late List<DateTime?> _flipStartTimes;
  late List<double> _rotates;
  late List<ui.Image?> _images;
  late List<Color> _colors;
  final Map<String, ui.Image> _imageCache = {};
  final ImageProxy _imageProxy = imageMemoryManager.requireProxy();
  Ticker? _ticker;
  Timer? _checkTimer;
  bool _isExecuting = false;

  @override
  void initState() {
    super.initState();
    _initializeGrid();
    if (widget.paths.length > 1) {
      _checkTimer = Timer.periodic(const Duration(seconds: 1), (timer) {
        if (!mounted) {
          timer.cancel();
          return;
        }
        if (!_isExecuting) {
          _check();
        }
      });
    }
  }

  int i = 0;

  double rand(double x) {
    i += 1;
    return hash13(
      x + i,
      (DateTime.now().millisecondsSinceEpoch / 100000).floor().toDouble(),
      widget.name.hashCode.toDouble(),
    );
  }

  late double pixelRatio;
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    pixelRatio = MediaQuery.devicePixelRatioOf(context);

    final int size = (widget.size / _gridCount).ceil();
    final targetSize = size * pixelRatio.ceil();

    for (int k = 0; k < _gridCount * _gridCount; k++) {
      _images[k] = _imageProxy.getCachedImage(_backPaths[k], targetSize);
    }

    if (_ticker == null) {
      _ticker = Ticker(_onTick);
      if (widget.paths.length > 1) {
        _updateCache();
      } else {
        _updateCache().then((_) {
          if (!mounted) return;
          setState(() {});
        });
      }
    }
  }

  @override
  void dispose() {
    super.dispose();
    _ticker?.dispose();
    _checkTimer?.cancel();
    _imageCache.clear();
    _imageProxy.dispose();
  }

  int _compareRandom0(String a, String b) {
    return rand(a.hashCode / b.hashCode) > 0.5 ? 1 : -1;
  }

  int _compareRandom1(String a, String b) {
    return rand(a.hashCode / b.hashCode + 1) > 0.5 ? 1 : -1;
  }

  List<Color> randomColorList() {
    return List.generate(
      _gridCount * _gridCount,
      (index) {
        return pickRandom(widget.colors, index);
      },
    );
  }

  void _initializeGrid() {
    _lastFlipTime = DateTime.now();
    _gridCount = _determineGridSize();
    _frontPaths = List.from(widget.paths);
    _backPaths = List.from(widget.paths);
    _frontPaths.sort(_compareRandom0);
    _backPaths.sort(_compareRandom1);
    _frontColors = randomColorList();
    _backColors = randomColorList();
    _isFront = List.filled(_gridCount * _gridCount, false);
    _isFlipping = List.filled(_gridCount * _gridCount, false);
    _flipStartTimes = List.filled(_gridCount * _gridCount, null);
    _rotates = List.filled(_gridCount * _gridCount, 0.0);
    _images = List.filled(_gridCount * _gridCount, null);
    _colors = List.from(_backColors);
  }

  int _determineGridSize() {
    if (widget.paths.length < 4) return 1;
    if (widget.paths.length < 9) return 2;
    return 3;
  }

  void _onTick(Duration elapsed) {
    _updateParameters();
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

    final List<String> pathsToLoad =
        currentPaths.where((path) => !_imageCache.containsKey(path)).toList();

    await Future.wait(pathsToLoad.map((path) => _loadAndCacheImage(path)));
  }

  Future<void> _loadAndCacheImage(String path) async {
    final int size = (widget.size / _gridCount).ceil();
    final targetSize = size * pixelRatio.ceil();

    final resizedImage = await _imageProxy.requestImage(path, targetSize);

    _imageCache[path] = resizedImage;

    if (!mounted) return;

    for (int k = 0; k < _gridCount * _gridCount; k++) {
      final currentPath =
          (_rotates[k] >= pi / 2) ? _frontPaths[k] : _backPaths[k];
      if (currentPath == path) {
        _images[k] = resizedImage;
      }
    }

    setState(() {});
  }

  void _stageFlipGridData(int index) {
    const int maxAttempts = 10;
    int attempts = 0;
    String newPath;

    do {
      newPath = pickRandom(widget.paths, index);
      attempts++;
    } while ((_frontPaths.contains(newPath) ||
            _backPaths.contains(newPath) ||
            _frontPaths[index] == newPath ||
            _backPaths[index] == newPath) &&
        attempts < maxAttempts);

    if (attempts < maxAttempts) {
      _backPaths[index] = newPath;
    }
  }

  void _prepareFlip() {
    bool isGoingToFlip = false;
    for (int k = 0; k < _gridCount * _gridCount; k++) {
      if (rand(k.toDouble()) > 0.64) {
        _isFlipping[k] = true;
        _flipStartTimes[k] = DateTime.now();
        _stageFlipGridData(k);
        isGoingToFlip = true;
      } else {
        _isFlipping[k] = false;
        _flipStartTimes[k] = null;
      }
    }
    if (isGoingToFlip && !(_ticker?.isActive ?? false)) {
      _ticker?.start();
    }
  }

  T pickRandom<T>(List<T> x, int k) {
    return x[(x.length * rand(k.toDouble())).floor()];
  }

  void _updateParameters() {
    bool needsUpdate = false;
    for (int k = 0; k < _gridCount * _gridCount; k++) {
      if (_isFlipping[k] && _flipStartTimes[k] != null) {
        final elapsedTime =
            DateTime.now().difference(_flipStartTimes[k]!).inMilliseconds;
        _rotates[k] = (elapsedTime / widget.speed) * pi;
        if (_rotates[k] > pi) {
          _rotates[k] = 0;
          _isFlipping[k] = false;
          _flipStartTimes[k] = null;
          _isFront[k] = !_isFront[k];

          final x = _frontPaths[k];
          _frontPaths[k] = _backPaths[k];
          _backPaths[k] = x;

          final y = _frontColors[k];
          _frontColors[k] = _backColors[k];
          _backColors[k] = y;
        }
        needsUpdate = true;
      } else {
        _rotates[k] = 0;
      }

      _images[k] =
          _imageCache[(_rotates[k] >= pi / 2) ? _frontPaths[k] : _backPaths[k]];

      _colors[k] = (_rotates[k] >= pi / 2) ? _frontColors[k] : _backColors[k];
    }

    if (needsUpdate) {
      if (!mounted) return;
      setState(() {});
    } else {
      if (_ticker?.isActive ?? false) {
        _ticker?.stop();
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return RepaintBoundary(
      child: CustomPaint(
        painter: FlipGridPainter(
          _images,
          gridCount: _gridCount,
          rotates: _rotates,
          fallbackColors: _colors,
        ),
        // Set the size to fill the available space
        size: Size.infinite,
      ),
    );
  }
}