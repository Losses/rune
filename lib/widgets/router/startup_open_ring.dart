import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

/// How long startup may take to open the last library again before a ring shows for it, so
/// that an open that takes no longer shows none.
const startupOpenRingDelay = Duration(milliseconds: 500);

/// The ring shown while startup still opens the last library again, once that has taken
/// [startupOpenRingDelay].
class StartupOpenRing extends StatefulWidget {
  const StartupOpenRing({super.key});

  @override
  State<StartupOpenRing> createState() => _StartupOpenRingState();
}

class _StartupOpenRingState extends State<StartupOpenRing> {
  late final Timer _delay;
  bool _shown = false;

  @override
  void initState() {
    super.initState();
    _delay = Timer(startupOpenRingDelay, () => setState(() => _shown = true));
  }

  @override
  void dispose() {
    _delay.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) =>
      _shown ? const Center(child: ProgressRing()) : const SizedBox.shrink();
}
