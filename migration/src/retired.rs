//! Migrations that builds made before the first release (July to September
//! 2024) applied under names that were later renamed or removed. They keep
//! their places in the migrator so a database that recorded them still
//! opens, and they do nothing: a database made since never needs them, and
//! for one that recorded them,
//! [`super::m20261123_000039_settle_pre_release_tables`] brings the tables
//! they made to the current ones.

use sea_orm_migration::prelude::*;

macro_rules! retired_migration {
    ($module:ident, $name:literal) => {
        pub mod $module {
            use super::*;

            pub struct Migration;

            impl MigrationName for Migration {
                fn name(&self) -> &str {
                    $name
                }
            }

            #[async_trait::async_trait]
            impl MigrationTrait for Migration {
                async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
                    Ok(())
                }

                async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
                    Ok(())
                }
            }
        }
    };
}

// Made `user_logs`, which nothing ever wrote to; removed on 2024-09-12.
retired_migration!(user_logs, "m20230701_000004_create_user_logs_table");
// Made `playlist_items`, renamed `media_file_playlists` on 2024-08-22.
retired_migration!(
    playlist_items,
    "m20230701_000006_create_playlist_items_table"
);
// Made `smart_playlists`, which nothing ever wrote to; removed on 2024-09-12.
retired_migration!(
    smart_playlists,
    "m20230701_000007_create_smart_playlists_table"
);
// Made `media_cover_arts`, renamed `media_cover_art` on 2024-07-27.
retired_migration!(
    media_cover_arts,
    "m20230728_000008_create_media_cover_arts_table"
);
