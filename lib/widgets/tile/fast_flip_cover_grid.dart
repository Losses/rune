import 'dart:math';
import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/scheduler.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'utils/image_proxy.dart';
import 'utils/flip_grid_painter.dart';
import 'constants/image_memory_manager.dart';

const flipInterval = 8;

class FastFlipCoverGrid extends StatefulWidget {
  final List<String> paths;
  final int speed;
  final int size;

  const FastFlipCoverGrid({
    super.key,
    required this.paths,
    required this.size,
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
  late List<bool> _isFront;
  late List<bool> _isFlipping;
  late List<DateTime?> _flipStartTimes;
  late List<double> _rotates;
  late List<ui.Image?> _images;
  final Random _random = Random();
  final Map<String, ui.Image> _imageCache = {};
  final ImageProxy _imageProxy = imageMemoryManager.requireProxy();
  Ticker? _ticker;
  bool _isExecuting = false;

  @override
  void initState() {
    super.initState();
    _initializeGrid();
  }

  late double pixelRatio;
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    pixelRatio = MediaQuery.devicePixelRatioOf(context);

    final int size = (widget.size / _gridCount).ceil();
    final targetSize = size * pixelRatio.ceil();

    _images = List.generate(
      _gridCount * _gridCount,
      (i) => _imageProxy.getCachedImage(_frontPaths[i], targetSize),
    );

    if (_ticker == null) {
      _updateCache();
      _ticker = Ticker(_onTick)..start();
    }
  }

  @override
  void dispose() {
    super.dispose();
    _ticker?.dispose();
    _imageCache.clear();
    _imageProxy.dispose();
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

    // Store the image in cache
    _imageCache[path] = resizedImage;

    if (!mounted) return;
    setState(() {});
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
      _backPaths[index] = newPath;
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

          final frontPath = _frontPaths[k];
          _frontPaths[k] = _backPaths[k];
          _backPaths[k] = frontPath;
        }
        needsUpdate = true;
      } else {
        _rotates[k] = 0;
      }

      _images[k] =
          _imageCache[(_rotates[k] >= pi / 2) ? _frontPaths[k] : _backPaths[k]];
    }

    if (needsUpdate) {
      if (!mounted) return;
      setState(() {});
    }
  }

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      painter: FlipGridPainter(
        _images,
        gridCount: _gridCount,
        rotates: _rotates,
      ),
      // Set the size to fill the available space
      size: Size.infinite,
    );
  }
}
