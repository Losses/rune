import 'package:fluent_ui/fluent_ui.dart';

import '../../messages/cover_art.pb.dart';

import 'widgets/random_grid.dart';

const int count = 40;

class LargeScreenCoverWallView extends StatefulWidget {
  const LargeScreenCoverWallView({super.key});

  @override
  State<LargeScreenCoverWallView> createState() =>
      _LargeScreenCoverWallViewState();
}

class _LargeScreenCoverWallViewState extends State<LargeScreenCoverWallView> {
  List<String> paths = [];
  bool _isLoading = true;

  @override
  void initState() {
    super.initState();

    _fetchRandomCoverArtIds();
  }

  Future<void> _fetchRandomCoverArtIds() async {
    GetRandomCoverArtIdsRequest(count: count).sendSignalToRust();
    GetRandomCoverArtIdsResponse.rustSignalStream.listen((event) {
      final response = event.message;

      if (!mounted) return;
      setState(() {
        paths = response.paths;
        _isLoading = false;
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    return _isLoading ? Container() : RandomGrid(seed: 42, paths: paths);
  }
}
