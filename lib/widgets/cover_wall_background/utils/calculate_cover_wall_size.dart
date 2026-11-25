import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

double calculateCoverWallGridSize(BoxConstraints constraints) => max(
      max(constraints.maxWidth, constraints.maxHeight) / 24,
      64,
    );
