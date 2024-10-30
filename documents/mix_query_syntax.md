# Documentation for Mix Query Syntax

## Purpose

The purpose of this query system is to enable complex querying of media files from a database. It supports:
- Filtering media files based on attributes such as artist, album, playlist, track, and directory.
- Sorting media files by attributes such as last modified date, duration, played through count, and skipped count.
- Applying additional filters like whether a media file is liked.
- Limiting the number of results returned.
- Generating recommendations based on certain criteria.

## Operators and Parameters

The query system supports a variety of operators, each accepting specific parameters. These operators are used to construct the query that retrieves the media files from the database.

| **Type**                | **Operator**                 | **Parameter**                  | **Description**                                                                 |
|-------------------------|----------------------------|---------------------------|--------------------------------------------------------------------------|
| **Filtering Operators** | **lib::artist**            | `i32` (Artist ID)         | Filters media files by the given artist ID.                              |
|                         | **lib::album**             | `i32` (Album ID)          | Filters media files by the given album ID.                               |
|                         | **lib::playlist**          | `i32` (Playlist ID)       | Filters media files by the given playlist ID.                            |
|                         | **lib::track**             | `i32` (Track ID)          | Filters media files by the given track ID.                               |
|                         | **lib::directory.deep**    | `String` (Directory Path) | Filters media files by the given directory path, including all subdirectories. |
|                         | **lib::directory.shallow** | `String` (Directory Path) | Filters media files by the given directory path, excluding subdirectories. |
| **Sorting Operators**   | **sort::track_number**     | `bool` (Ascending/Descending) | Sorts media files by their disk and track number. `true` for ascending, `false` for descending. |
|                         | **sort::last_modified**    | `bool` (Ascending/Descending) | Sorts media files by their last modified date. `true` for ascending, `false` for descending. |
|                         | **sort::duration**         | `bool` (Ascending/Descending) | Sorts media files by their duration. `true` for ascending, `false` for descending. |
|                         | **sort::playedthrough**    | `bool` (Ascending/Descending) | Sorts media files by their played through count. `true` for ascending, `false` for descending. |
|                         | **sort::skipped**          | `bool` (Ascending/Descending) | Sorts media files by their skipped count. `true` for ascending, `false` for descending. |
| **Filtering by Liked Status** | **filter::liked**            | `bool` (Liked/Not Liked)  | Filters media files by their liked status. `true` for liked, `false` for not liked. |
|                               | **filter::with_cover_art**   | `bool` (With/Without)     | Filters media files by cover art existence. `true` for with cover arts, `false` for without cover arts. |
| **Limiting and Recommendation Operators** | **pipe::limit**  | `u64` (Limit) | Limits the number of media files returned by the query.                  |
|                         | **pipe::recommend**        | `i32` (Recommendation Group) | Generates recommendations based on the given recommendation group.       |
| **Unknown Operator**    | **Unknown**                | `String` (Operator Name)  | Represents an unknown operator. It is used for logging and debugging purposes. |

## Query Process

The query process involves several steps to ensure that the desired media files are retrieved efficiently and accurately. The steps are as follows:

### 1. Parsing the Queries

Each query provided by the user is parsed using the `parse_query` function. This function converts the query into a `QueryOperator` enum, which represents the type of operation to be performed along with its parameter.

### 2. Initializing Conditions and Variables

Several variables are initialized to store the IDs and parameters for filtering and sorting. These include:
- `artist_ids`, `album_ids`, `playlist_ids`, `track_ids` for storing IDs.
- `directories_deep`, `directories_shallow` for storing directory paths.
- `sort_last_modified_asc`, `sort_duration_asc`, `sort_playedthrough_asc`, `sort_skipped_asc` for sorting options.
- `filter_liked` for the liked status filter.
- `pipe_limit` for limiting the number of results.
- `pipe_recommend` for generating recommendations.

### 3. Filtering by Library Attributes

The query system first applies filters based on library attributes (artist, album, playlist, track, and directory). It creates an OR condition that combines all the subconditions for these attributes. This ensures that media files matching any of the specified criteria are included in the results.

### 4. Applying Additional Filters

If a liked status filter is provided, it is applied to the query. The query is also joined with the `media_file_stats` table if sorting by played through count, skipped count, or filtering by liked status is required.

### 5. Sorting the Results

The query system applies sorting based on the provided sorting options. It uses the `apply_sorting_macro` and `apply_cursor_sorting_macro` macros to handle the sorting logic.

### 6. Handling Recommendations

If a recommendation group is specified, the query system generates recommendations based on the provided group. It retrieves the virtual point for the recommendation and fetches the recommended media files.

### 7. Applying Limit and Pagination

The query system applies the limit to the number of results if specified. It also handles cursor-based pagination to fetch the appropriate page of results.

### 8. Executing the Query

Finally, the constructed query is executed against the database to retrieve the media files. The results are returned to the user.

By following these steps, the query system efficiently retrieves and returns the desired media files based on the user's criteria.

## CLI Tool Usage

Rune provides a command-line interface (CLI) tool to help developers and users quickly debug and test queries in the Rune database. By using the Mix subcommand, users can verify their query statements and retrieve corresponding media file results. Here are some usage examples:

### Examples

1. **Filter and Sort Media Files**

   This command retrieves all media files, filters out those without cover art, and sorts them by track number in ascending order:

   ```bash
   rune-cli ~/Music/ mix -m "lib::all(true);filter::with_cover_art(false);sort::track_number(true);"
   ```

2. **Generate Recommendations**

   This command generates media file recommendations based on recommendation group 8:

   ```bash
   rune-cli ~/Music/ mix -m "pipe::recommend(8)"
   ```

3. **Directory Filtering, Sorting, Recommendation, and Limiting Results**

   This command retrieves media files from the "workout" directory and all its subdirectories, sorts them by duration in descending order, generates recommendations from group 1, and limits the results to 5:

   ```bash
   rune-cli ~/Music/ mix -m "lib::directory.deep(workout);sort::duration(false);pipe::recommend(1);pipe::limit(5)"
   ```

### Usage Instructions

- **Path Parameter**: `~/Music/` indicates the root directory where media files are located. Adjust this path according to your file location.
- **Query Syntax**: Use the `-m` parameter to specify the query statement, which can include multiple operators such as filtering, sorting, recommending, and limiting.
- **Operator Combination**: Users can combine different operators as needed to perform complex queries.

These commands allow users to flexibly test and debug database queries, optimizing media file retrieval and recommendations.

## Recommendation Group

The `recommendation_group` parameter in the `pipe::recommend` operator is used to generate recommendations based on percentile analysis of media file attributes. This process involves the following steps:

### 1. Retrieving Percentile Analysis

The `get_percentile_analysis_result` function calculates the percentile values for various attributes of media files. These attributes include audio features such as RMS, ZCR, energy, spectral centroid, and others.

### 2. Calculating the Virtual Point

The `get_percentile` function calculates the rank and retrieves the value for a given percentile. This value represents the attribute value at the specified percentile.

### 3. Generating Recommendations

Using the virtual point calculated from the percentile analysis, the system fetches media files that are similar to this virtual point. The number of recommendations generated can be limited by the `pipe::limit` parameter.