import 'dart:async';
import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../config/animation.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/responsive_providers.dart';
import '../../utils/api/fetch_artist_page.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/l10n.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../utils/execute_middle_click_action.dart';
import '../../utils/rune_log.dart';
import '../../utils/query_list.dart';
import '../../utils/router/artist_section_parameter.dart';
import '../../utils/router/navigation.dart';
import '../../widgets/artist_page/release_arrangements.dart';
import '../../widgets/artist_page/release_spans.dart';
import '../../widgets/artist_page/release_tile.dart';
import '../../widgets/belt_container.dart';
import '../../widgets/hover_opacity.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';
import '../../widgets/start_screen/start_group.dart';
import '../../widgets/start_screen/start_group_implementation.dart';
import '../../widgets/start_screen/start_group_item.dart' show Dimensions;
import '../../widgets/tile_group/tile_layout.dart';
import '../../widgets/track_list/band_screen_track_list.dart';
import '../../widgets/track_list/large_screen_track_list.dart';
import '../../widgets/track_list/small_screen_track_list.dart';
import '../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../widgets/turntile/turntile_group.dart';
import '../../widgets/turntile/turntile_group_items_tile.dart';
import '../../widgets/turntile/turntile_normal_layout.dart';
import '../../widgets/start_screen/stacked_layout.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../query_tracks/query_tracks.dart';
import '../query_tracks/query_tracks_list.dart';

/// The name a section of an artist goes by: the library's own when it gives
/// one, else the name of its kind.
String artistSectionName(AppLocalizations s, ArtistSection section) {
  if (section.title.isNotEmpty) return section.title;
  return switch (section.kind) {
    ArtistSectionKind.albums => s.albums,
    ArtistSectionKind.eps ||
    ArtistSectionKind.singles => s.artistSectionSinglesAndEps,
    ArtistSectionKind.compilations => s.artistSectionCompilations,
    ArtistSectionKind.live => s.artistSectionLive,
    ArtistSectionKind.appearsOn => s.artistSectionAppearsOn,
    ArtistSectionKind.latestRelease => s.artistSectionLatestRelease,
    ArtistSectionKind.essentialAlbums => s.artistSectionEssentialAlbums,
  };
}

/// The route a section opens, as the library says it opens.
String artistSectionRoute(ArtistSection section) => switch (section.opens) {
  ArtistSectionOpens.album when section.album != null => albumDetailRoute,
  ArtistSectionOpens.albums => artistSectionAlbumsRoute,
  _ => artistSectionTracksRoute,
};

/// The links to all the [sections] of an artist, in their order. The header's
/// row above the page of a section has those that open a page of songs or
/// albums, as the library's row has only its lists.
List<ArtistSectionLink> artistSectionLinks(
  AppLocalizations s,
  List<ArtistSection> sections,
  String artistName,
) {
  final targets = <ArtistSectionRowLink>[
    for (final section in sections)
      (
        title: artistSectionName(s, section),
        route: artistSectionRoute(section),
        queries: QueryList.fromMixQuery(section.queries),
      ),
  ];
  final row = [
    for (final target in targets)
      if (target.route != albumDetailRoute) target,
  ];
  final links = <ArtistSectionLink>[];
  var index = 0;
  for (final (i, target) in targets.indexed) {
    final album = sections[i].album;
    final Object arguments;
    if (target.route == albumDetailRoute && album != null) {
      arguments = ArtistAlbumParameter(album.id, album.name);
    } else {
      arguments = ArtistSectionParameter(
        target.queries,
        target.title,
        artistName,
        sections: row,
        index: index,
      );
      index += 1;
    }
    links.add((title: target.title, route: target.route, arguments: arguments));
  }
  return links;
}

/// The page of an artist: tiles for its release sections and the tracks.
class ArtistPage extends StatefulWidget {
  final int artistId;
  final String name;

  /// Asks the library for the page, in the language the app shows. Only a
  /// test gives another one.
  final Future<FetchArtistPageResponse> Function(int artistId, String language)
  fetch;

  /// Tells when the library's content changed. Only a test gives another one.
  final Stream<void>? contentChanged;

  /// Picks how the circles of the release tiles are laid out. Only a test
  /// gives one.
  final math.Random? random;

  const ArtistPage({
    super.key,
    required this.artistId,
    required this.name,
    this.fetch = fetchArtistPage,
    this.contentChanged,
    this.random,
  });

  @override
  State<ArtistPage> createState() => _ArtistPageState();
}

class _ArtistPageState extends State<ArtistPage> {
  // Picked once for a visit of the page, so that the tiles keep their looks
  // while the page is rebuilt or laid out anew.
  late ReleaseArrangements _arrangements = ReleaseArrangements(widget.random);

  // The release tiles enter when the page arrives, the tracks when their
  // first page does.
  final _releasesLayoutManager = StartScreenLayoutManager();
  final _layoutManager = StartScreenLayoutManager();
  // The release tiles and the track list keep their state when the window
  // crosses the width at which the large page starts to scroll sideways.
  final _releasesKey = GlobalKey();
  final _tracksKey = GlobalKey();
  StreamSubscription<void>? _contentSubscription;
  FetchArtistPageResponse? _response;
  Object? _failure;
  int _generation = 0;
  int _inFlight = 0;
  bool _refreshPending = false;
  Timer? _entranceTimer;
  // The language the page was last asked in.
  String? _language;

  @override
  void initState() {
    super.initState();
    // Covers may arrive after the page, so the page is asked again without
    // taking down what it shows.
    _contentSubscription =
        (widget.contentChanged ?? LibraryContentChanged.rustSignalStream)
            .listen((_) => _refresh());
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // The library names the sections in the language the app shows, so the
    // page is asked first here, and again when that language changes.
    final language = Localizations.localeOf(context).toLanguageTag();
    if (language == _language) return;
    final first = _language == null;
    _language = language;
    if (first) {
      _load(quiet: false);
    } else {
      _refresh();
    }
  }

  @override
  void didUpdateWidget(covariant ArtistPage oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.artistId == widget.artistId) return;
    _arrangements = ReleaseArrangements(widget.random);
    _load(quiet: false);
  }

  /// Asks again quietly. Events that come while a request is out make one more
  /// request after it, not one each.
  void _refresh() {
    if (_inFlight > 0) {
      _refreshPending = true;
      return;
    }
    _load(quiet: true);
  }

  Future<void> _load({required bool quiet}) async {
    final generation = ++_generation;
    _inFlight++;
    if (!quiet) {
      setState(() {
        _response = null;
        _failure = null;
      });
    }
    try {
      final response = await widget.fetch(widget.artistId, _language!);
      if (!mounted || generation != _generation) return;
      if (!quiet) _replayReleases();
      setState(() {
        _response = response;
        _failure = null;
      });
    } catch (error) {
      if (!mounted || generation != _generation) return;
      // A page that is already shown stays when only the refresh fails.
      if (quiet && _response != null) {
        warn$('Refreshing the artist page failed: $error');
        return;
      }
      setState(() => _failure = error);
    } finally {
      _inFlight--;
      if (_inFlight == 0 && _refreshPending && mounted) {
        _refreshPending = false;
        _load(quiet: true);
      }
    }
  }

  /// Plays the entrance of the release tiles the page brings in.
  void _replayReleases() {
    _releasesLayoutManager.resetAnimations();
    _entranceTimer?.cancel();
    _entranceTimer = Timer(
      Duration(milliseconds: gridAnimationDelay),
      _releasesLayoutManager.playAnimations,
    );
  }

  @override
  void dispose() {
    _contentSubscription?.cancel();
    _entranceTimer?.cancel();
    _releasesLayoutManager.dispose();
    _layoutManager.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: _layoutManager,
      child: DeviceTypeBuilder(
        // The classes the library home tells apart. Every other class is
        // laid out as the home's large list is.
        deviceType: const [
          DeviceType.band,
          DeviceType.belt,
          DeviceType.dock,
          DeviceType.zune,
          DeviceType.tv,
        ],
        builder: (context, deviceType) {
          final isMini =
              deviceType == DeviceType.dock || deviceType == DeviceType.band;
          // The classes the collection pages give StartScreen.
          final isLarge =
              deviceType != DeviceType.band &&
              deviceType != DeviceType.belt &&
              deviceType != DeviceType.dock &&
              deviceType != DeviceType.zune;
          return PageContentFrame(
            top: isMini,
            // As the library home's large list has it, which pads its groups
            // itself.
            left: !isLarge,
            // The name is at the top, below the status bar.
            child: SafeArea(
              left: false,
              right: false,
              bottom: false,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (!isMini)
                    // The name flies up into the title of the header when a
                    // page that names the artist opens, and back, as the
                    // link of a library page does.
                    QueryTracksTitle(
                      title: widget.name,
                      flipKey: 'child:$artistDetailRoute',
                    ),
                  Expanded(
                    child: Builder(
                      builder: (context) => isLarge
                          ? _largeContent(context)
                          : _listContent(context, deviceType),
                    ),
                  ),
                ],
              ),
            ),
          );
        },
      ),
    );
  }

  Widget? _failureOrRing(BuildContext context) {
    final failure = _failure;
    if (failure != null) {
      return Center(child: Text(requestErrorText(S.of(context), failure)));
    }
    if (_response == null) return const Center(child: ProgressRing());
    return null;
  }

  /// How many covers the first section has, none counted twice.
  static int _firstCovers(FetchArtistPageResponse response) =>
      response.sections.isEmpty
      ? 0
      : response.sections.first.covers.toSet().length;

  /// Builds the tile of the section at an index, of the size of its span.
  Widget Function(int index, TileSpan span) _releaseTiles(
    BuildContext context,
    FetchArtistPageResponse response,
  ) {
    final links = artistSectionLinks(
      S.of(context),
      response.sections,
      widget.name,
    );
    return (index, span) {
      final section = response.sections[index];
      final link = links[index];
      final queries = QueryList.fromMixQuery(section.queries);
      return ReleaseTile(
        label: link.title,
        covers: section.covers,
        cells: span.columns,
        arrangement: _arrangements.of(
          index,
          span.columns,
          section.covers.toSet().length,
        ),
        onPressed: () => $push(link.route, arguments: link.arguments),
        onContextMenu: (context, position, attachKey, controller) =>
            openQueryListContextMenu(
              position,
              context,
              attachKey,
              controller,
              queries,
            ),
        onMiddleClick: () =>
            executeQueryListMiddleClickAction(this.context, queries),
      );
    };
  }

  Widget _withReleasesLayout(Widget child) =>
      ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: _releasesLayoutManager,
        child: child,
      );

  /// Counts the rows as the groups of the home do, up to the most the release
  /// tiles are set for.
  static Dimensions _releaseDimensions(
    double containerHeight,
    double cellSize,
    double gapSize,
    List items,
  ) {
    final rows = math.min(
      maxReleaseShortSide,
      StartGroupImplementation.startLinkDimensionCalculator(
        containerHeight,
        cellSize,
        gapSize,
        items,
      ).rows,
    );
    return Dimensions(
      rows: rows,
      columns: (items.length / rows).ceil(),
      count: items.length,
    );
  }

  /// The page as the library home's large list has its groups: Releases and
  /// Tracks side by side. When the window is too narrow for Releases and one
  /// column of Tracks, the row scrolls sideways.
  Widget _largeContent(BuildContext context) {
    final early = _failureOrRing(context);
    if (early != null) return early;
    final response = _response!;
    final s = S.of(context);
    final count = response.sections.length;
    final hasTracks = response.tracks.isNotEmpty;
    if (count == 0 && !hasTracks) {
      return Center(child: Text(s.noDataAvailable));
    }
    final padding = getScrollContainerPadding(context);
    final tile = _releaseTiles(context, response);

    return LayoutBuilder(
      builder: (context, constraints) {
        final height = constraints.maxHeight - padding.top - padding.bottom;
        final spans = releaseSpans(
          count,
          _releaseDimensions(
            height,
            defaultCellSize,
            defaultGapSize,
            response.sections,
          ).rows,
          _firstCovers(response),
        );
        final releases = count == 0
            ? null
            : StartGroup<int>(
                groupIndex: 0,
                groupTitle: s.releases,
                items: [for (var i = 0; i < count; i += 1) i],
                constraints: BoxConstraints(
                  maxWidth: constraints.maxWidth - padding.left - padding.right,
                  maxHeight: height,
                ),
                groupLayoutVariation: StartGroupGroupLayoutVariation.stacked,
                dimensionCalculator: _releaseDimensions,
                gapSize: defaultGapSize,
                strip: 1,
                tileSpanBuilder: (index) => spans[index],
                itemBuilder: (context, index) => tile(index, spans[index]),
                direction: Axis.horizontal,
              );

        // One column of the track list, with the paddings it puts round it.
        final minTracks = hasTracks
            ? StartGroupStackedLayout.widthFor(
                context,
                s.tracks,
                padding.horizontal +
                    largeTrackListRightPlus +
                    largeTrackListTileWidth,
              )
            : 0.0;
        final fits =
            (releases?.widthIn(context) ?? 0) + minTracks <=
            constraints.maxWidth - padding.left;

        final Widget? tracks = hasTracks
            ? StartGroupStackedLayout(
                key: _tracksKey,
                groupTitle: s.tracks,
                child: QueryTrackListView(
                  layoutManager: _layoutManager,
                  queries: QueryList.fromMixQuery(response.tracks),
                  mode: null,
                  layout: DeviceType.tv,
                  underTitle: true,
                ),
              )
            : null;
        final children = [
          if (releases != null)
            KeyedSubtree(
              key: _releasesKey,
              child: _withReleasesLayout(releases),
            ),
          if (tracks != null)
            fits
                ? Expanded(child: tracks)
                : SizedBox(width: minTracks, height: height, child: tracks),
        ];

        if (fits) {
          return Padding(
            padding: EdgeInsets.only(
              left: padding.left,
              bottom: padding.bottom,
            ),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: children,
            ),
          );
        }
        return SmoothHorizontalScroll(
          builder: (context, scrollController) => SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            controller: scrollController,
            padding: EdgeInsets.only(
              left: padding.left,
              bottom: padding.bottom,
            ),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: children,
            ),
          ),
        );
      },
    );
  }

  /// The page in the classes the library home draws as a strip, as one
  /// upright list, or as the turntile screen: the releases lead one list with
  /// the tracks.
  Widget _listContent(BuildContext context, DeviceType deviceType) {
    final early = _failureOrRing(context);
    if (early != null) return early;
    final response = _response!;
    final hasTracks = response.tracks.isNotEmpty;

    // With nothing to lead it, an empty list says so itself.
    final header = response.sections.isEmpty && !hasTracks
        ? null
        : _withReleasesLayout(
            deviceType == DeviceType.zune
                ? _turntileHeader(context, response)
                : _stripHeader(
                    context,
                    response,
                    deviceType == DeviceType.dock
                        ? Axis.vertical
                        : Axis.horizontal,
                  ),
          );

    if (hasTracks) {
      return QueryTrackListView(
        layoutManager: _layoutManager,
        queries: QueryList.fromMixQuery(response.tracks),
        mode: null,
        header: header,
      );
    }
    // No tracks: the same lists, with the header alone.
    final list = deviceType == DeviceType.zune
        ? SmallScreenTrackList(
            totalCount: 0,
            getItem: (_) => null,
            queries: const QueryList(),
            mode: null,
            reloadData: () {},
            header: header,
          )
        : BandScreenTrackList(
            totalCount: 0,
            getItem: (_) => null,
            queries: const QueryList(),
            mode: null,
            reloadData: () {},
            header: header,
          );
    return deviceType == DeviceType.belt ? BeltContainer(child: list) : list;
  }

  /// The releases as a tile group of the turntile screen, and the title of
  /// the tracks.
  Widget _turntileHeader(
    BuildContext context,
    FetchArtistPageResponse response,
  ) {
    final theme = FluentTheme.of(context);
    final s = S.of(context);
    final count = response.sections.length;
    final tile = _releaseTiles(context, response);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (count > 0)
          LayoutBuilder(
            builder: (context, constraints) {
              final columns =
                  TurntileGroupItemsTile.defaultTurntileDimensionCalculator(
                    constraints.maxWidth -
                        TurnTileGroupNormalLayout.padding.horizontal,
                    TurntileGroup.tileCellSize,
                    TurntileGroup.defaultGapSize,
                    response.sections,
                  ).columns;
              final spans = releaseSpans(
                count,
                math.min(maxReleaseShortSide, columns),
                _firstCovers(response),
              );
              return TurntileGroup<int>(
                groupIndex: 0,
                groupTitle: s.releases,
                items: [for (var i = 0; i < count; i += 1) i],
                gridLayoutVariation: TurntileGroupGridLayoutVariation.tile,
                gapSize: TurntileGroup.defaultGapSize,
                tileSpanBuilder: (index) => spans[index],
                itemBuilder: (context, index) => tile(index, spans[index]),
              );
            },
          ),
        if (response.tracks.isNotEmpty)
          // The title of a group of the turntile screen, as
          // TurnTileGroupNormalLayout draws it over its items.
          Padding(
            padding: EdgeInsets.only(
              left: TurnTileGroupNormalLayout.padding.left,
              right: TurnTileGroupNormalLayout.padding.right,
            ),
            child: HoverOpacity(
              child: Padding(
                padding: TurnTileGroupNormalLayout.titlePadding,
                child: Text(s.tracks, style: theme.typography.bodyLarge),
              ),
            ),
          ),
      ],
    );
  }

  /// One tile per section, as the band screen draws the tile of an item in
  /// `BandScreen.buildList`.
  Widget _stripHeader(
    BuildContext context,
    FetchArtistPageResponse response,
    Axis axis,
  ) {
    final tile = _releaseTiles(context, response);
    final tiles = [
      for (var i = 0; i < response.sections.length; i += 1)
        ManagedTurntileScreenItem(
          groupId: 0,
          row: i,
          column: 1,
          child: AspectRatio(
            aspectRatio: 1,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 1),
              child: tile(i, const TileSpan(1, 1)),
            ),
          ),
        ),
    ];
    return FocusTraversalGroup(
      child: axis == Axis.horizontal
          ? Row(mainAxisSize: MainAxisSize.min, children: tiles)
          : Column(mainAxisSize: MainAxisSize.min, children: tiles),
    );
  }
}
