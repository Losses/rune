//! Library databases as builds made before the first release left them, for
//! tests. Each holds the same library: two files, the first with an artist,
//! an album, analysis and (once builds had it) cover art, and a playlist of
//! the second then the first.

use sea_orm_migration::sea_orm::{ConnectionTrait, DatabaseConnection, DbErr};

/// The pre-release table layouts, by the day the build that made them was
/// current.
#[derive(Clone, Copy, Debug)]
pub enum PreReleaseLibrary {
    /// The first: no cover art, artists or albums yet, analysis with the
    /// sample rate and duration, playlists without a group, playlist items
    /// with `file_id`.
    July13,
    /// Cover art in `media_cover_arts`, which `media_files` points to.
    July27,
    /// Artists and albums without a group, the sample rate and duration on
    /// `media_files`.
    August6,
    /// As the last pre-release builds left a library: groups, a nullable
    /// playlist group, `playlist_items` with `media_file_id`.
    August26,
}

impl PreReleaseLibrary {
    pub const ALL: [PreReleaseLibrary; 4] = [
        PreReleaseLibrary::July13,
        PreReleaseLibrary::July27,
        PreReleaseLibrary::August6,
        PreReleaseLibrary::August26,
    ];

    /// Whether the build had cover art, artists and albums.
    pub fn has_cover_art(self) -> bool {
        !matches!(self, PreReleaseLibrary::July13)
    }

    pub fn has_artists(self) -> bool {
        matches!(
            self,
            PreReleaseLibrary::August6 | PreReleaseLibrary::August26
        )
    }

    fn migrations(self) -> Vec<&'static str> {
        let mut recorded = vec![
            "m20230701_000001_create_media_files_table",
            "m20230701_000002_create_media_metadata_table",
            "m20230701_000003_create_media_analysis_table",
            "m20230701_000004_create_user_logs_table",
            "m20230701_000005_create_playlists_table",
            "m20230701_000006_create_playlist_items_table",
            "m20230701_000007_create_smart_playlists_table",
        ];
        match self {
            PreReleaseLibrary::July13 => {}
            PreReleaseLibrary::July27 => {
                recorded.push("m20230728_000008_create_media_cover_arts_table");
            }
            PreReleaseLibrary::August6 | PreReleaseLibrary::August26 => recorded.extend([
                "m20230728_000008_create_media_cover_art_table",
                "m20230806_000009_create_artists_table",
                "m20230806_000010_create_media_file_artists_table",
                "m20230806_000011_create_albums_table",
                "m20230806_000012_create_media_file_albums_table",
            ]),
        }
        recorded
    }

    fn tables(self) -> Vec<String> {
        let analysis_extra = match self {
            PreReleaseLibrary::July13 | PreReleaseLibrary::July27 => {
                r#""sample_rate" integer NOT NULL, "duration" real NOT NULL, "#
            }
            _ => "",
        };
        let chroma: String = (0..12)
            .map(|index| format!(r#", "chroma{index}" real"#))
            .collect();
        let mut tables = vec![
            r#"CREATE TABLE "seaql_migrations" ( "version" text NOT NULL PRIMARY KEY, "applied_at" bigint NOT NULL )"#.to_owned(),
            r#"CREATE TABLE "media_metadata" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "file_id" integer NOT NULL, "meta_key" text NOT NULL, "meta_value" text NOT NULL, FOREIGN KEY ("file_id") REFERENCES "media_files" ("id") ON DELETE CASCADE ON UPDATE CASCADE )"#
            .to_owned(),
            format!(
                r#"CREATE TABLE "media_analysis" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "file_id" integer NOT NULL, {analysis_extra}"spectral_centroid" real, "spectral_flatness" real, "spectral_slope" real, "spectral_rolloff" real, "spectral_spread" real, "spectral_skewness" real, "spectral_kurtosis" real{chroma}, FOREIGN KEY ("file_id") REFERENCES "media_files" ("id") ON DELETE CASCADE ON UPDATE CASCADE )"#
            ),
            r#"CREATE TABLE "user_logs" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "file_id" integer NOT NULL, "listen_time" text NOT NULL, "progress" real NOT NULL, FOREIGN KEY ("file_id") REFERENCES "media_files" ("id") ON DELETE CASCADE ON UPDATE CASCADE )"#.to_owned(),
            r#"CREATE TABLE "smart_playlists" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "name" text NOT NULL, "query" text NOT NULL, "created_at" text NOT NULL, "updated_at" text NOT NULL )"#.to_owned(),
        ];
        let file_column = match self {
            PreReleaseLibrary::August26 => "media_file_id",
            _ => "file_id",
        };
        tables.push(format!(
            r#"CREATE TABLE "playlist_items" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "playlist_id" integer NOT NULL, "{file_column}" integer NOT NULL, "position" integer NOT NULL, FOREIGN KEY ("playlist_id") REFERENCES "playlists" ("id") ON DELETE CASCADE ON UPDATE CASCADE, FOREIGN KEY ("{file_column}") REFERENCES "media_files" ("id") ON DELETE CASCADE ON UPDATE CASCADE )"#
        ));
        tables.push(match self {
            PreReleaseLibrary::August26 => r#"CREATE TABLE "playlists" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "name" text NOT NULL, "group" text NULL, "created_at" text NOT NULL, "updated_at" text NOT NULL )"#.to_owned(),
            _ => r#"CREATE TABLE "playlists" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "name" text NOT NULL, "created_at" text NOT NULL, "updated_at" text NOT NULL )"#.to_owned(),
        });
        match self {
            PreReleaseLibrary::July13 => tables.push(
                r#"CREATE TABLE "media_files" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "file_name" text NOT NULL, "directory" text NOT NULL, "extension" text NOT NULL, "file_hash" text(64) NOT NULL, "last_modified" text NOT NULL )"#.to_owned(),
            ),
            PreReleaseLibrary::July27 => tables.extend([
                r#"CREATE TABLE "media_cover_arts" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "file_hash" text(64) NOT NULL, "binary" binary(16777216) NOT NULL )"#.to_owned(),
                r#"CREATE TABLE "media_files" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "file_name" text NOT NULL, "directory" text NOT NULL, "extension" text NOT NULL, "file_hash" text(64) NOT NULL, "last_modified" text NOT NULL, "cover_art_id" integer NULL, FOREIGN KEY ("cover_art_id") REFERENCES "media_cover_arts" ("id") ON DELETE CASCADE ON UPDATE CASCADE )"#.to_owned(),
            ]),
            PreReleaseLibrary::August6 | PreReleaseLibrary::August26 => {
                let group = match self {
                    PreReleaseLibrary::August26 => r#", "group" text NOT NULL"#,
                    _ => "",
                };
                let track_number = match self {
                    PreReleaseLibrary::August26 => r#", "track_number" integer NULL"#,
                    _ => "",
                };
                tables.extend([
                    r#"CREATE TABLE "media_cover_art" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "file_hash" text(64) NOT NULL, "binary" binary(16777216) NOT NULL )"#.to_owned(),
                    r#"CREATE TABLE "media_files" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "file_name" text NOT NULL, "directory" text NOT NULL, "extension" text NOT NULL, "file_hash" text(64) NOT NULL, "last_modified" text NOT NULL, "cover_art_id" integer NULL, "sample_rate" integer NOT NULL, "duration" real NOT NULL, FOREIGN KEY ("cover_art_id") REFERENCES "media_cover_art" ("id") ON DELETE CASCADE ON UPDATE CASCADE )"#.to_owned(),
                    format!(r#"CREATE TABLE "artists" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "name" text NOT NULL UNIQUE{group} )"#),
                    r#"CREATE TABLE "media_file_artists" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "media_file_id" integer NOT NULL, "artist_id" integer NOT NULL, FOREIGN KEY ("media_file_id") REFERENCES "media_files" ("id") ON DELETE CASCADE ON UPDATE CASCADE, FOREIGN KEY ("artist_id") REFERENCES "artists" ("id") ON DELETE CASCADE ON UPDATE CASCADE )"#.to_owned(),
                    format!(r#"CREATE TABLE "albums" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "name" text NOT NULL UNIQUE{group} )"#),
                    format!(r#"CREATE TABLE "media_file_albums" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "media_file_id" integer NOT NULL UNIQUE{track_number}, "album_id" integer NOT NULL, FOREIGN KEY ("media_file_id") REFERENCES "media_files" ("id") ON DELETE CASCADE ON UPDATE CASCADE, FOREIGN KEY ("album_id") REFERENCES "albums" ("id") ON DELETE CASCADE ON UPDATE CASCADE )"#),
                ]);
            }
        }
        tables
    }

    fn rows(self) -> Vec<String> {
        let (media_file_columns, media_file_values) = match self {
            PreReleaseLibrary::July13 => ("", ["", ""]),
            PreReleaseLibrary::July27 => (", cover_art_id", [", 1", ", NULL"]),
            _ => (
                ", cover_art_id, sample_rate, duration",
                [", 1, 44100, 61.5", ", NULL, 48000, 3.25"],
            ),
        };
        let mut rows = match self {
            PreReleaseLibrary::July13 => Vec::new(),
            PreReleaseLibrary::July27 => vec![
                "INSERT INTO media_cover_arts (id, file_hash, binary) VALUES (1, 'cover', x'89504e47')"
                    .to_owned(),
            ],
            PreReleaseLibrary::August6 | PreReleaseLibrary::August26 => vec![
                "INSERT INTO media_cover_art (id, file_hash, binary) VALUES (1, 'cover', x'89504e47')"
                    .to_owned(),
            ],
        };
        rows.extend([
            format!(
                "INSERT INTO media_files (id, file_name, directory, extension, file_hash, last_modified{media_file_columns}) VALUES (1, 'Intro.flac', '/music', 'flac', '{hash}', '2024-07-01 10:00:00'{first}), (2, 'Outro.flac', '/music', 'flac', '{hash}', '2024-07-01 10:00:00'{second})",
                hash = "0".repeat(64),
                first = media_file_values[0],
                second = media_file_values[1],
            ),
            "INSERT INTO media_metadata (file_id, meta_key, meta_value) VALUES (1, 'title', 'Intro'), (1, 'artist', 'Ärzte'), (2, 'title', 'Outro')".to_owned(),
        ]);
        rows.push(match self {
            PreReleaseLibrary::July13 | PreReleaseLibrary::July27 => {
                "INSERT INTO media_analysis (file_id, sample_rate, duration, spectral_centroid) VALUES (1, 44100, 61.5, 1.5)".to_owned()
            }
            _ => "INSERT INTO media_analysis (file_id, spectral_centroid) VALUES (1, 1.5)".to_owned(),
        });
        rows.push(match self {
            PreReleaseLibrary::August26 => "INSERT INTO playlists (id, name, \"group\", created_at, updated_at) VALUES (1, 'Favourites', NULL, '2024-08-01', '2024-08-01')".to_owned(),
            _ => "INSERT INTO playlists (id, name, created_at, updated_at) VALUES (1, 'Favourites', '2024-08-01', '2024-08-01')".to_owned(),
        });
        let file_column = match self {
            PreReleaseLibrary::August26 => "media_file_id",
            _ => "file_id",
        };
        rows.push(format!(
            "INSERT INTO playlist_items (playlist_id, {file_column}, position) VALUES (1, 2, 0), (1, 1, 1)"
        ));
        if self.has_artists() {
            {
                let (group_column, group_value) = match self {
                    PreReleaseLibrary::August26 => (", \"group\"", ", 'A'"),
                    _ => ("", ""),
                };
                rows.push(format!(
                    "INSERT INTO artists (id, name{group_column}) VALUES (1, 'Ärzte'{group_value})"
                ));
                let album_group = match self {
                    PreReleaseLibrary::August26 => ", 'G'",
                    _ => "",
                };
                rows.push(format!(
                    "INSERT INTO albums (id, name{group_column}) VALUES (1, 'Greatest'{album_group})"
                ));
                // Pre-release scans could link a file to an artist twice.
                rows.push(
                    "INSERT INTO media_file_artists (media_file_id, artist_id) VALUES (1, 1), (1, 1)"
                        .to_owned(),
                );
                rows.push(
                    "INSERT INTO media_file_albums (media_file_id, album_id) VALUES (1, 1)"
                        .to_owned(),
                );
            }
        }
        rows
    }
}

/// Fill the empty database [db] with the library as the build of [layout]
/// left it.
pub async fn write_pre_release_library(
    db: &DatabaseConnection,
    layout: PreReleaseLibrary,
) -> Result<(), DbErr> {
    for statement in layout.tables() {
        db.execute_unprepared(&statement).await?;
    }
    for version in layout.migrations() {
        db.execute_unprepared(&format!(
            "INSERT INTO seaql_migrations (version, applied_at) VALUES ('{version}', 1724676287)"
        ))
        .await?;
    }
    for statement in layout.rows() {
        db.execute_unprepared(&statement).await?;
    }
    Ok(())
}
