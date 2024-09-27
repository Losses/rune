import 'package:fluent_ui/fluent_ui.dart';

import './fancy_cover_config.dart';

const config1 = [
  FancyCoverConfig(
    fontSize: 48,
    position: Offset(0.5, 0),
    fontWeight: FontWeight.w500,
    transformOrigin: Offset(0, -0.5),
  ),
  FancyCoverConfig(
    fontSize: 66,
    position: Offset(0.03, 0.14),
    fontWeight: FontWeight.w100,
    transformOrigin: Offset(-0.5, -0.5),
  ),
  FancyCoverConfig(
    fontSize: 36,
    position: Offset(0, 0.38),
    fontWeight: FontWeight.w900,
    transformOrigin: Offset(-0.5, -0.5),
    toUpperCase: true,
  ),
];

const config2 = [
  FancyCoverConfig(
    fontSize: 48,
    position: Offset(0.0, 0.4),
    fontWeight: FontWeight.w200,
    rotation: -45,
  ),
  FancyCoverConfig(
    fontSize: 48,
    position: Offset(0.2, 0.55),
    fontWeight: FontWeight.w900,
    rotation: -45,
  ),
  FancyCoverConfig(
    fontSize: 32,
    position: Offset(-0.2, 0.8),
    fontWeight: FontWeight.w100,
    toUpperCase: true,
    rotation: -45,
    textBoxWidth: 0.5,
    textAlign: TextAlign.end,
  ),
];

const config3 = [
  FancyCoverConfig(
    fontSize: 48,
    position: Offset(0.5, 0.02),
    fontWeight: FontWeight.w800,
    rotation: 90,
    transformOrigin: Offset(0, 0.5),
  ),
  FancyCoverConfig(
    fontSize: 48,
    position: Offset(0.7, 0.5),
    fontWeight: FontWeight.w400,
    textAlign: TextAlign.end,
    textBoxWidth: 0.6,
  ),
  FancyCoverConfig(
    fontSize: 20,
    position: Offset(0.75, 0.9),
    fontWeight: FontWeight.w100,
    toUpperCase: true,
    textBoxWidth: 0.5,
    textAlign: TextAlign.end,
  ),
];

const configs = [config1, config2, config3];
