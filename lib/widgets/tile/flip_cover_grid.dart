import 'dart:async';
import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/foundation.dart' show listEquals;
import 'package:flip_card/flip_card.dart';
import 'package:flip_card/flip_card_controller.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import './cover_art.dart';

class FlipCoverGrid extends StatefulWidget {
  final List<String> paths;
  final String id;
  final int speed;
  final BoringAvatarType emptyTileType;

  const FlipCoverGrid({
    super.key,
    required this.id,
    required this.paths,
    this.speed = 500,
    this.emptyTileType = BoringAvatarType.bauhaus,
  });

  @override
  FlipCoverGridState createState() => FlipCoverGridState();
}

class FlipCoverGridState extends State<FlipCoverGrid> {
  Timer? _timer;
  final Random _random = Random();
  List<FlipCardController> _controllers = const [];
  late List<String> _frontPaths;
  late List<String> _backPaths;
  late int _gridSize;
  late int hash;

  bool _needFlip() {
    final n = widget.paths.length;
    return n != 0 && n != 1 && n != 4 && n != 9;
  }

  @override
  void initState() {
    super.initState();
    _showPaths();
  }

  @override
  void didUpdateWidget(covariant FlipCoverGrid oldWidget) {
    super.didUpdateWidget(oldWidget);
    // A search result keeps its grid while its covers arrive, so the tiles, their flip
    // controllers and the flip timer follow the current paths.
    if (!listEquals(oldWidget.paths, widget.paths)) {
      _showPaths();
    }
  }

  void _showPaths() {
    _initializeNumbers();
    _gridSize = _determineGridSize(widget.paths.length);
    hash = widget.paths.join(':').hashCode;
    _controllers = widget.paths.isEmpty
        ? const []
        : List.generate(_gridSize * _gridSize, (_) => FlipCardController());
    _timer?.cancel();
    _timer = _needFlip() ? _startTimer() : null;
  }

  void _initializeNumbers() {
    _frontPaths = List.from(widget.paths);
    _backPaths = List.from(widget.paths);

    _frontPaths.shuffle();
    _backPaths.shuffle();
  }

  Timer _startTimer() {
    return Timer.periodic(const Duration(seconds: 8), (timer) {
      if (widget.paths.length > 1) {
        for (int i = 0; i < _controllers.length; i++) {
          if (_random.nextDouble() > 0.64) {
            _controllers[i].toggleCard();
          }
        }
      }
    });
  }

  void _replaceNumber(int index) {
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
      setState(() {
        if (_controllers[index].state?.isFront == true) {
          _backPaths[index] = newPath;
        } else {
          _frontPaths[index] = newPath;
        }
      });
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final colors = [
      theme.accentColor,
      theme.accentColor.light,
      theme.accentColor.lighter,
      theme.accentColor.lightest,
      theme.accentColor.dark,
      theme.accentColor.darker,
      theme.accentColor.darkest,
    ];

    if (widget.paths.isEmpty) {
      return BoringAvatar(
        name: widget.id,
        palette: BoringAvatarPalette(colors),
        type: widget.emptyTileType,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(0),
        ),
      );
    }

    return LayoutBuilder(
      builder: (context, constraints) {
        final tileSize = constraints.maxWidth / _gridSize;

        return Stack(
          children: List.generate(_gridSize * _gridSize, (index) {
            final row = index ~/ _gridSize;
            final col = index % _gridSize;

            return Positioned(
              left: col * tileSize,
              top: row * tileSize,
              width: tileSize,
              height: tileSize,
              child: FlipCard(
                speed: widget.speed,
                direction: FlipDirection.VERTICAL,
                controller: _controllers[index],
                flipOnTouch: false,
                onFlipDone: (isFront) {
                  _replaceNumber(index);
                },
                front: _buildCard(
                    _frontPaths[index], index + hash),
                back:
                    _buildCard(_backPaths[index], index + hash),
              ),
            );
          }),
        );
      },
    );
  }

  int _determineGridSize(int length) {
    if (length < 4) return 1;
    if (length < 9) return 2;
    return 3;
  }

  Widget _buildCard(String? path, int hash) {
    return RepaintBoundary(
      child: SizedBox(
        width: double.infinity,
        height: double.infinity,
        child: Center(
          child: CoverArt(
            hash: hash,
            path: path,
          ),
        ),
      ),
    );
  }
}
