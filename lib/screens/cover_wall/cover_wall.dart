import 'package:fluent_ui/fluent_ui.dart';

import './widgets/cover_wall.dart';

class CoverWallPage extends StatefulWidget {
  const CoverWallPage({super.key});

  @override
  State<CoverWallPage> createState() => _CoverWallPageState();
}

class _CoverWallPageState extends State<CoverWallPage> {
  @override
  Widget build(BuildContext context) {
    return const CoverWallView();
  }
}
