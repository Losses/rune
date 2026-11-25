import '../tile_group/tile_layout.dart';

/// The most cells the short side of a release group has.
const int maxReleaseShortSide = 3;

/// The fewest covers a section has for a tile of two by two cells: as many
/// as the circles in sight of the arrangement with one large circle among
/// small ones, which only such a tile has.
const int largeReleaseTileCovers = 6;

/// The tiles of [count] release sections in order, for a tile group that
/// places them with strip 1 and whose short side has [shortSide] cells: its
/// rows when it grows sideways, its columns when it grows downward. The first
/// section, which has [firstCovers] covers, none counted twice, is a tile of
/// two by two cells where the short side has room for it and the section has
/// [largeReleaseTileCovers] covers or more. Every other section is one cell,
/// as a group of the home ends its last line where its tiles run out.
List<TileSpan> releaseSpans(int count, int shortSide, int firstCovers) => [
  for (var i = 0; i < count; i += 1)
    i == 0 && shortSide >= 2 && firstCovers >= largeReleaseTileCovers
        ? const TileSpan(2, 2)
        : const TileSpan(1, 1),
];
