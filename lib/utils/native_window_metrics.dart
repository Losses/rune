import 'package:flutter/widgets.dart';

/// Convert OS window logical lengths into the scaled UI coordinate space.
/// Native window decorations retain the system DPR, while MediaQuery describes
/// the application's effective DPR inside AppViewport.
double nativeWindowLogicalToLocal(BuildContext context, double length) {
  return length *
      View.of(context).devicePixelRatio /
      MediaQuery.devicePixelRatioOf(context);
}
