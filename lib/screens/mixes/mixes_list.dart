import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../utils/router_extra.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../widgets/flip_tile.dart';
import '../../widgets/grouped_list_base.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/start_screen/start_screen.dart';
import '../../messages/mix.pbserver.dart';

class MixesListView extends GroupedListBase<Mix, MixesGroupSummary> {
  const MixesListView({super.key});

  @override
  MixesListViewState createState() => MixesListViewState();
}

class MixesListViewState extends GroupedListBaseState<Mix, MixesGroupSummary> {
  @override
  Future<List<Group<Mix>>> fetchSummary() async {
    final fetchArtistsGroupSummary = FetchMixesGroupSummaryRequest();
    fetchArtistsGroupSummary.sendSignalToRust(); // GENERATED

    final rustSignal = await MixGroupSummaryResponse.rustSignalStream.first;
    final mixGroupList = rustSignal.message;

    return mixGroupList.mixesGroups.map((summary) {
      return Group<Mix>(
        groupTitle: summary.groupTitle,
        items: [], // Initially empty, will be filled in fetchPage
      );
    }).toList();
  }

  @override
  Future<List<Group<Mix>>> fetchGroups(List<String> groupTitles) async {
    final fetchArtistsGroupsRequest = FetchMixesGroupsRequest()
      ..groupTitles.addAll(groupTitles);
    fetchArtistsGroupsRequest.sendSignalToRust(); // GENERATED

    final rustSignal = await MixesGroups.rustSignalStream.first;
    final mixesGroups = rustSignal.message.groups;

    return mixesGroups.map((group) {
      return Group<Mix>(
        groupTitle: group.groupTitle,
        items: group.mixes,
      );
    }).toList();
  }

  @override
  Widget itemBuilder(BuildContext context, Mix item) {
    return MixItem(
      mix: item,
      refresh: pagingController.refresh,
    );
  }
}

class MixItem extends StatelessWidget {
  final Mix mix;
  final void Function() refresh;

  MixItem({
    super.key,
    required this.mix,
    required this.refresh,
  });

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  @override
  Widget build(BuildContext context) {
    return ContextMenuWrapper(
      contextAttachKey: contextAttachKey,
      contextController: contextController,
      onContextMenu: (position) {
        openCollectionItemContextMenu(
          position,
          context,
          contextAttachKey,
          contextController,
          'mix',
          mix.id,
          refresh,
          mix.locked,
        );
      },
      child: FlipTile(
        name: mix.name,
        coverIds: mix.coverIds,
        emptyTileType: BoringAvatarType.bauhaus,
        onPressed: () {
          context.push('/mixes/${mix.id}', extra: QueryTracksExtra(mix.name));
        },
      ),
    );
  }
}
