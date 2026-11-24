//! Live smoke check against the real API. Read-only.
//!
//! ```text
//! AM_DEV_TOKEN=... [AM_USER_TOKEN=...] [AM_STOREFRONT=us] \
//!     cargo run -p apple-music --example smoke
//! ```
//!
//! Without `AM_USER_TOKEN` only catalog calls run. With it, the user's
//! storefront, the first library songs, their catalog ids, ratings and the
//! recently added list are fetched. Nothing is written.

use std::env;
use std::sync::Arc;

use futures_util::{StreamExt, TryStreamExt};

use apple_music::models::CatalogSearchType;
use apple_music::{AppleMusicClient, StaticTokens};

#[tokio::main]
async fn main() -> apple_music::Result<()> {
    let developer = env::var("AM_DEV_TOKEN").expect("set AM_DEV_TOKEN");
    let user = env::var("AM_USER_TOKEN").ok().filter(|t| !t.is_empty());
    let has_user = user.is_some();
    let storefront = env::var("AM_STOREFRONT").unwrap_or_else(|_| "us".into());

    let client = AppleMusicClient::builder(Arc::new(StaticTokens::new(developer, user)))
        .storefront(storefront)
        .build();

    let found = client
        .search_catalog("daft punk get lucky", &[CatalogSearchType::Songs], 3, 0)
        .await?;
    println!("search: {} songs", found.songs.data.len());
    let catalog_ids: Vec<String> = found.songs.data.iter().map(|s| s.id.clone()).collect();
    let songs = client.catalog_songs(&catalog_ids).await?;
    for song in &songs {
        let attrs = song.attributes.as_ref();
        println!(
            "  {} {} - {}",
            song.id,
            attrs.and_then(|a| a.artist_name.as_deref()).unwrap_or("?"),
            attrs.and_then(|a| a.name.as_deref()).unwrap_or("?"),
        );
    }

    if !has_user {
        println!("no AM_USER_TOKEN: skipping library calls");
        return Ok(());
    }

    let storefront = client.user_storefront().await?;
    println!("user storefront: {}", storefront.id);

    let mut library: Vec<_> = client
        .library_song_pages(&[], 1)
        .take(1)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .collect();
    library.truncate(5);
    println!("library songs (first {}):", library.len());
    for song in &library {
        println!(
            "  {} catalog={:?} {}",
            song.id,
            song.catalog_id(),
            song.attributes
                .as_ref()
                .and_then(|a| a.name.as_deref())
                .unwrap_or("?"),
        );
    }
    let library_ids: Vec<String> = library.iter().map(|s| s.id.clone()).collect();
    if !library_ids.is_empty() {
        let ratings = client.library_song_ratings(&library_ids).await?;
        println!("library ratings: {}", ratings.len());
        for r in ratings {
            println!("  {} = {:?}", r.id, r.attributes.and_then(|a| a.value));
        }
    }
    let ratings = client.catalog_song_ratings(&catalog_ids).await?;
    println!("catalog ratings: {}", ratings.len());

    let recent: Vec<_> = client.recently_added().take(5).try_collect().await?;
    println!("recently added (first {}): {recent:#?}", recent.len());

    let playlists: Vec<_> = client.library_playlists().take(5).try_collect().await?;
    for p in &playlists {
        let a = p.attributes.as_ref();
        println!(
            "playlist {} {:?} canEdit={:?} hasCatalog={:?}",
            p.id,
            a.and_then(|a| a.name.as_deref()),
            a.and_then(|a| a.can_edit),
            a.and_then(|a| a.has_catalog),
        );
    }
    Ok(())
}
