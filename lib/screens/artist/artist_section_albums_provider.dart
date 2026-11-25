import '../../bindings/bindings.dart';
import '../../utils/api/fetch_artist_section_albums.dart';
import '../../utils/api/fetch_collection_by_ids.dart';
import '../../utils/query_list.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../collection/utils/collection_data_provider.dart';

/// How many albums one page of a section holds.
const artistSectionAlbumsPageSize = 24;

/// How many albums the request for a section's group titles covers, which is
/// all of them.
const _artistSectionSummaryAlbums = 0x7fffffff;

typedef FetchSectionAlbumGroups =
    Future<List<ArtistSectionAlbumGroup>> Function(
      List<MixQuery> queries,
      int cursor,
      int pageSize,
    );

typedef FetchAlbumsByIds =
    Future<List<Collection>> Function(CollectionType type, List<int> ids);

/// Feeds the albums screens with the albums of one section of an artist's
/// page, a page of albums at a time, in the groups and order the library gives
/// them. A group a page ends with goes on in the next page's first group of
/// the same title. The summary, which the year picker lists, is the titles of
/// all the groups. A group the library leaves untitled, the one group of a
/// curated section, is titled [title].
class ArtistSectionAlbumsProvider extends CollectionDataProvider {
  /// The section's name, as its link shows it.
  final String title;

  ArtistSectionAlbumsProvider({
    required QueryList queries,
    required this.title,
    super.contentChanged,
    FetchSectionAlbumGroups fetchAlbumGroups = fetchArtistSectionAlbums,
    FetchAlbumsByIds fetchAlbums = _fetchAlbums,
  }) : super(
         collectionType: CollectionType.album,
         useCache: false,
         loadSummary: (_) async {
           final groups = await fetchAlbumGroups(
             queries.toQueryList(),
             0,
             _artistSectionSummaryAlbums,
           );
           return [
             for (final group in groups)
               Group<InternalCollection>(
                 groupTitle: group.groupTitle.isEmpty
                     ? title
                     : group.groupTitle,
                 items: [],
               ),
           ];
         },
         loadPage: (type, _, cursor) async {
           final groups = await fetchAlbumGroups(
             queries.toQueryList(),
             cursor * artistSectionAlbumsPageSize,
             artistSectionAlbumsPageSize,
           );
           final ids = [for (final group in groups) ...group.albumIds];
           final albums = ids.isEmpty
               ? <Collection>[]
               : await fetchAlbums(type, ids);
           final byId = {for (final album in albums) album.id: album};
           return (
             [
               for (final group in groups)
                 if (group.albumIds.any(byId.containsKey))
                   Group<InternalCollection>(
                     groupTitle: group.groupTitle.isEmpty
                         ? title
                         : group.groupTitle,
                     items: [
                       for (final id in group.albumIds)
                         if (byId[id] case final album?)
                           InternalCollection.fromRawCollection(album),
                     ],
                   ),
             ],
             ids.length < artistSectionAlbumsPageSize,
           );
         },
       );

  static Future<List<Collection>> _fetchAlbums(
    CollectionType type,
    List<int> ids,
  ) =>
      // With their covers, as the albums page fetches its groups.
      fetchCollectionByIds(type, ids, bakeCoverArts: true);

  @override
  void appendGroups(
    List<Group<InternalCollection>> groups,
    List<Group<InternalCollection>> page,
  ) {
    for (final group in page) {
      if (groups.isNotEmpty && groups.last.groupTitle == group.groupTitle) {
        final last = groups.removeLast();
        groups.add(
          Group<InternalCollection>(
            groupTitle: last.groupTitle,
            items: [...last.items, ...group.items],
          ),
        );
      } else {
        groups.add(group);
      }
    }
  }
}
