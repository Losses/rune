import 'package:fluent_ui/fluent_ui.dart';

import '../../../../widgets/ax_pressure.dart';

import 'turnstile_animation.dart';

const globalPadding = EdgeInsets.all(4.0);
const singleSize = 80.0;
const doubleSize = 160.0;

class TurntileExperiment extends StatelessWidget {
  const TurntileExperiment({
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 320,
      child: TurnstileAnimation(
        tiles: [
          Padding(
            padding: globalPadding,
            child: AxPressure(
              child: Container(
                width: singleSize,
                height: singleSize,
                color: Colors.red,
              ),
            ),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: singleSize, height: singleSize, color: Colors.blue),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: singleSize, height: singleSize, color: Colors.green),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: singleSize, height: singleSize, color: Colors.white),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: doubleSize, height: singleSize, color: Colors.green),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: singleSize, height: singleSize, color: Colors.red),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: singleSize, height: singleSize, color: Colors.blue),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: doubleSize, height: singleSize, color: Colors.blue),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: singleSize, height: singleSize, color: Colors.green),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: singleSize, height: singleSize, color: Colors.white),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: doubleSize, height: singleSize, color: Colors.red),
          ),
          Padding(
            padding: globalPadding,
            child: Container(
                width: doubleSize, height: singleSize, color: Colors.blue),
          ),
          // Add more tiles as needed
        ],
        enterMode: EnterMode.enter,
        yDirection: YDirection.bottomToTop,
        zDirection: ZDirection.frontToBack,
        duration: const Duration(milliseconds: 1000),
      ),
    );
  }
}
