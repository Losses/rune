use database::entities::{media_cover_art, media_cover_art::Column};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::time as tokio_time;

use anyhow::{Error, Result};
use futures::future::join_all;

use tokio::task;

use ::database::playing_item::dispatcher::PlayingItemActionDispatcher;
use ::playback::player::RemoteAudioError;

use crate::{
    Session, Signal,
    handlers::playback::{
        ItemRoutingParams, item_service, item_service_binding, local_item_library,
        playing_item_from_request, queue_file_ids, reject_remote_independent_item,
    },
    library_pool::LibraryHandle,
    messages::*,
    request_error::ExpectedRequestError,
    service::{ServiceBinding, covers::cover_primary_color},
    utils::{GlobalParams, ParamsExtractor, query_cover_arts, require_library},
};
use std::collections;

impl ParamsExtractor for GetCoverArtIdsByMixQueriesRequest {
    type Params = (Option<LibraryHandle>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(),)
    }
}

impl Signal for GetCoverArtIdsByMixQueriesRequest {
    type Params = (Option<LibraryHandle>,);
    type Response = GetCoverArtIdsByMixQueriesResponse;
    async fn handle(
        &self,
        (library,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;
        let main_db = Arc::clone(&library.main_db);
        let recommend_db = Arc::clone(&library.recommend_db);
        let queries = &request.requests;
        let queue = Arc::new(queue_file_ids(&request.queue.items, &library).await?);

        let files_futures = queries.clone().into_iter().map(|x| {
            let main_db = Arc::clone(&main_db);
            let recommend_db = Arc::clone(&recommend_db);
            let queue = queue.clone();
            async move {
                let query = query_cover_arts(
                    &main_db,
                    recommend_db,
                    x.queries.clone(),
                    Some(request.n),
                    &queue,
                )
                .await;

                match query {
                    Ok(files) => {
                        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
                        let uuids: collections::BTreeSet<_> = files
                            .iter()
                            .filter_map(|file| file.cover_art_uuid.as_deref())
                            .collect();
                        let mut cover_art_ids: Vec<i32> = media_cover_art::Entity::find()
                            .select_only()
                            .column(Column::Id)
                            .filter(Column::SourceUuid.is_in(uuids))
                            .into_tuple()
                            .all(&*main_db)
                            .await
                            .unwrap_or_default();

                        cover_art_ids.dedup();

                        GetCoverArtIdsByMixQueriesResponseUnit {
                            id: x.id,
                            cover_art_ids,
                        }
                    }
                    Err(_) => GetCoverArtIdsByMixQueriesResponseUnit {
                        id: x.id,
                        cover_art_ids: Vec::new(),
                    },
                }
            }
        });

        Ok(Some(GetCoverArtIdsByMixQueriesResponse {
            call_id: None,
            result: join_all(files_futures).await,
        }))
    }
}

/// How long a request for the color of a streaming item waits for its cover to download.
const COVER_COLOR_WAIT: Duration = Duration::from_secs(15);

/// The primary color of the cover of a streaming library's item, waiting up to
/// `COVER_COLOR_WAIT` for a cover that is not cached yet. None when the item has no cover, or
/// the service could not give it in time, as when offline. A service that rejects the
/// library's sign-in fails it, as it fails every request about the library's items.
async fn streaming_primary_color(
    binding: &dyn ServiceBinding,
    item: &PlayingItemRef,
) -> Result<Option<i32>> {
    let PlayingItemRef::LibraryTrack { track, .. } = item else {
        return Ok(None);
    };
    primary_color_of_cover_within(binding.cover_when_cached(track), COVER_COLOR_WAIT)
        .await
        .map_err(|error| {
            Error::new(ExpectedRequestError::ServiceSignInRequired {
                record_id: binding.library_id().to_string(),
            })
            .context(format!("{error:#}"))
        })
}

/// The primary color of a cover that `cover` gives the file of, once it does within `wait`.
/// A cover that cannot be had has no color, unless the service rejected the sign-in it was
/// asked with: that failure is returned.
async fn primary_color_of_cover_within(
    cover: impl Future<Output = Result<Option<String>>>,
    wait: Duration,
) -> Result<Option<i32>> {
    let cover = match tokio_time::timeout(wait, cover).await {
        Ok(Ok(Some(cover))) => cover,
        Ok(Ok(None)) | Err(_) => return Ok(None),
        Ok(Err(error))
            if error.chain().any(|cause| {
                matches!(
                    cause.downcast_ref::<RemoteAudioError>(),
                    Some(RemoteAudioError::CredentialsRejected)
                )
            }) =>
        {
            return Err(error);
        }
        Ok(Err(error)) => {
            log::warn!("Could not get the cover of a streaming item for its color: {error:#}");
            return Ok(None);
        }
    };
    Ok(cover_primary_color(&cover).await)
}

impl ParamsExtractor for GetPrimaryColorByTrackIdRequest {
    type Params = ItemRoutingParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            all_params.node.clone(),
            all_params.library_pool.clone(),
        )
    }
}

impl Signal for GetPrimaryColorByTrackIdRequest {
    type Params = ItemRoutingParams;
    type Response = GetPrimaryColorByTrackIdResponse;

    async fn handle(
        &self,
        (library, node, pool): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let Some(item) = dart_signal.item.clone() else {
            require_library(library)?;
            return Ok(None);
        };
        reject_remote_independent_item(session.as_ref(), &item)?;
        // A streaming library's item takes its color from its cover in that library's cover
        // cache, found by its own binding. Whether the item is one is read from its record.
        if item_service(&node, library.as_ref(), &item)?.is_some() {
            // A library that cannot be reached has no color to give. One that needs a new
            // sign-in says so, as every request about its items does.
            let primary_color = match item_service_binding(&node, library.as_ref(), &item).await {
                Ok(Some(binding)) => streaming_primary_color(binding.as_ref(), &item).await?,
                Ok(None) => None,
                Err(error)
                    if matches!(
                        error.downcast_ref::<ExpectedRequestError>(),
                        Some(ExpectedRequestError::ServiceSignInRequired { .. })
                    ) =>
                {
                    return Err(error);
                }
                Err(error) => {
                    log::warn!(
                        "Could not reach the library of a streaming item for its color: {error:#}"
                    );
                    None
                }
            };
            return Ok(Some(GetPrimaryColorByTrackIdResponse {
                call_id: None,
                item,
                primary_color,
            }));
        }
        let library = local_item_library(library, &pool, &item).await?;
        let parsed_item = playing_item_from_request(item.clone(), Some(&library)).await?;
        let response = task::spawn_blocking(move || {
            let runtime = Runtime::new().unwrap();
            runtime.block_on(async move {
                let dispatcher = PlayingItemActionDispatcher::new();
                let primary_color = dispatcher
                    .get_cover_art_primary_color(
                        &library.fsio,
                        Path::new(&library.media_root),
                        &library.main_db,
                        &parsed_item,
                    )
                    .await;

                GetPrimaryColorByTrackIdResponse {
                    call_id: None,
                    item,
                    primary_color,
                }
            })
        })
        .await?;

        Ok(Some(response))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use futures::future::BoxFuture;
    use tempfile::TempDir;
    use tokio::time as tokio_time;

    use super::primary_color_of_cover_within;
    use crate::service::covers::{CoverSource, ServiceCoverCache, cover_primary_color};
    use crate::utils::{Broadcaster, RinfRustSignal};

    const WAIT: Duration = Duration::from_secs(5);

    /// A 2 by 2 bitmap of one color.
    fn bitmap(rgb: [u8; 3]) -> Vec<u8> {
        let mut bytes = b"BM".to_vec();
        bytes.extend(70_u32.to_le_bytes());
        bytes.extend([0; 4]);
        bytes.extend(54_u32.to_le_bytes());
        bytes.extend(40_u32.to_le_bytes());
        bytes.extend(2_i32.to_le_bytes());
        bytes.extend(2_i32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(24_u16.to_le_bytes());
        bytes.extend([0; 24]);
        for _ in 0..2 {
            // Two pixels in blue, green, red order, and the row padded to four bytes.
            bytes.extend([rgb[2], rgb[1], rgb[0], rgb[2], rgb[1], rgb[0], 0, 0]);
        }
        bytes
    }

    struct Quiet;

    impl Broadcaster for Quiet {
        fn broadcast(&self, _: &dyn RinfRustSignal) {}
    }

    /// Serves one cover after `delay`, or fails, and counts its downloads.
    struct Source {
        bytes: Vec<u8>,
        delay: Duration,
        fails: bool,
        downloads: AtomicUsize,
    }

    impl CoverSource for Source {
        fn cover_bytes<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
            Box::pin(async move {
                self.downloads.fetch_add(1, Ordering::SeqCst);
                tokio_time::sleep(self.delay).await;
                if self.fails {
                    Err(anyhow!("the service is out of reach"))
                } else {
                    Ok(self.bytes.clone())
                }
            })
        }
    }

    fn source(delay: Duration, fails: bool) -> Arc<Source> {
        Arc::new(Source {
            bytes: bitmap([200, 30, 30]),
            delay,
            fails,
            downloads: AtomicUsize::new(0),
        })
    }

    fn cache_in(root: &TempDir) -> Arc<ServiceCoverCache> {
        Arc::new(ServiceCoverCache::new(
            root.path().to_path_buf(),
            Arc::new(Quiet),
        ))
    }

    #[tokio::test]
    async fn a_cached_cover_gives_the_color_the_local_path_gives_for_its_bytes() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = source(Duration::ZERO, false);
        cache
            .ensure_cached(Arc::clone(&source), "cover")
            .await
            .unwrap();
        let expected = metadata::cover_art::get_primary_color(&source.bytes);
        assert!(expected.is_some());

        let color =
            primary_color_of_cover_within(cache.ensure_cached(Arc::clone(&source), "cover"), WAIT)
                .await
                .unwrap();
        assert_eq!(color, expected);
        assert_eq!(source.downloads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_cover_not_cached_yet_gives_its_color_after_the_download() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = source(Duration::from_millis(150), false);
        assert!(cache.cached_path("cover").await.unwrap().is_none());

        let color =
            primary_color_of_cover_within(cache.ensure_cached(Arc::clone(&source), "cover"), WAIT)
                .await
                .unwrap();
        assert_eq!(color, metadata::cover_art::get_primary_color(&source.bytes));
        assert!(color.is_some());
    }

    #[tokio::test]
    async fn a_failing_or_too_slow_download_gives_no_color() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let failing = source(Duration::ZERO, true);
        let color =
            primary_color_of_cover_within(cache.ensure_cached(Arc::clone(&failing), "cover"), WAIT)
                .await
                .unwrap();
        assert_eq!(color, None);

        let slow = source(Duration::from_secs(60), false);
        let color = primary_color_of_cover_within(
            cache.ensure_cached(Arc::clone(&slow), "other"),
            Duration::from_millis(50),
        )
        .await
        .unwrap();
        assert_eq!(color, None);

        // A track with no cover has no color either.
        let color = primary_color_of_cover_within(async { Ok(None) }, WAIT)
            .await
            .unwrap();
        assert_eq!(color, None);
    }

    #[tokio::test]
    async fn the_color_is_kept_beside_the_cover_and_read_from_there() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = source(Duration::ZERO, false);
        let cover = cache
            .ensure_cached(Arc::clone(&source), "cover")
            .await
            .unwrap()
            .unwrap();
        let color = cover_primary_color(&cover).await.unwrap();
        let kept = std::fs::read_to_string(Path::new(&cover).with_extension("color")).unwrap();
        let (kept_for, kept_color) = kept.trim().split_once(' ').unwrap();
        assert_eq!(kept_color.parse::<i32>().unwrap(), color);

        // A later read takes it from the file without decoding the cover again.
        std::fs::write(
            Path::new(&cover).with_extension("color"),
            format!("{kept_for} 42"),
        )
        .unwrap();
        assert_eq!(cover_primary_color(&cover).await, Some(42));
        // A cover that is not an image has no color, and none is kept for it.
        std::fs::write(root.path().join("junk.img"), b"junk").unwrap();
        let junk = root.path().join("junk.img").to_str().unwrap().to_owned();
        assert_eq!(cover_primary_color(&junk).await, None);
        assert!(!root.path().join("junk.color").exists());
    }

    #[tokio::test]
    async fn a_color_kept_for_other_artwork_is_not_given_for_the_cover() {
        let root = TempDir::new().unwrap();
        let cover = root.path().join("cover.img");
        std::fs::write(&cover, bitmap([200, 30, 30])).unwrap();
        let cover = cover.to_str().unwrap().to_owned();
        let red = cover_primary_color(&cover).await.unwrap();
        let kept = std::fs::read(Path::new(&cover).with_extension("color")).unwrap();
        // The cover is evicted and fetched again with other artwork, and a worker that was still
        // working out the old cover's color writes it beside the new one.
        std::fs::write(&cover, bitmap([30, 30, 200])).unwrap();
        std::fs::write(Path::new(&cover).with_extension("color"), kept).unwrap();
        let blue = cover_primary_color(&cover).await;
        assert_ne!(blue, Some(red));
        assert_eq!(
            blue,
            metadata::cover_art::get_primary_color(&bitmap([30, 30, 200]))
        );
    }
}
