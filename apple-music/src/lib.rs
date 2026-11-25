//! Async client for the Apple Music API (`api.music.apple.com`).
//!
//! The client is platform independent: tokens come from a [`TokenSource`]
//! implemented by the host application. It covers the user's library, catalog
//! songs and search, and song ratings, with typed models, paging, request
//! retries and a cap on concurrent requests.
//!
//! ```no_run
//! use std::sync::Arc;
//! use apple_music::{AppleMusicClient, StaticTokens};
//! use futures_util::TryStreamExt;
//!
//! # async fn demo() -> apple_music::Result<()> {
//! let tokens = Arc::new(StaticTokens::new("developer-jwt", Some("user-token".into())));
//! let client = AppleMusicClient::new(tokens);
//! let artists: Vec<_> = client.library_artists().try_collect().await?;
//! println!("{} artists", artists.len());
//! # Ok(())
//! # }
//! ```

mod api;
mod client;
mod error;
pub mod models;
#[cfg(feature = "test-fixtures")]
pub mod test_fixtures;
mod tokens;

pub use client::{AppleMusicClient, ClientBuilder, DEFAULT_BASE_URL, RetryPolicy};
pub use error::{ApiError, Error, Result};
pub use tokens::{StaticTokens, TokenSource};

/// Largest number of catalog song ids per request.
pub const CATALOG_SONGS_BATCH: usize = 300;
/// Largest number of catalog album ids per request.
pub const CATALOG_ALBUMS_BATCH: usize = 100;
/// Largest number of library song ids per request.
pub const LIBRARY_SONGS_BATCH: usize = 300;
/// Page size used for library listings.
pub const LIBRARY_PAGE_SIZE: u32 = 100;
/// Ids per request for rating lookups. Apple documents no limit for these, so
/// this keeps URLs short.
pub const ID_BATCH: usize = 100;
/// Largest `limit` of a library search.
pub const LIBRARY_SEARCH_LIMIT: u32 = 25;
/// Largest `limit` of a catalog search, per type.
pub const CATALOG_SEARCH_LIMIT: u32 = 25;
/// Page size of recently added resources, Apple's largest.
pub const RECENTLY_ADDED_LIMIT: u32 = 25;
/// Pages of recently added resources that are read.
pub const RECENTLY_ADDED_MAX_PAGES: usize = 4;
/// Pages of a catalog artist's albums that are read; a longer list is cut and logged.
pub const CATALOG_ARTIST_ALBUMS_MAX_PAGES: usize = 20;
/// Groups per page of recommendations.
pub const RECOMMENDATIONS_LIMIT: u32 = 10;
/// Pages of recommendations that are read; later ones are of little use.
pub const RECOMMENDATIONS_MAX_PAGES: usize = 3;
