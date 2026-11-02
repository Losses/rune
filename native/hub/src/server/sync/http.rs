//! Audio content endpoints mounted by an authenticated caller.
use anyhow::Result;
use axum::{Router, extract::State, http::StatusCode, routing::get};
use database::sync::LibrarySyncStore;
use database::sync::model::{BlobRef, Collection, ObjectKey, Payload};
use database::{
    actions::{downloads, file},
    entities,
};
use std::{sync::Arc, time::Duration};

/// The application replaces a library only while its background workers are stopped.
#[derive(Clone)]
pub struct SharedLibrary(pub Arc<tokio::sync::RwLock<LibrarySyncStore>>);

/// Context for the audio routes. Mount these routes only after authenticating the peer.
#[derive(Clone)]
pub struct BlobContext {
    pub store: Arc<tokio::sync::RwLock<LibrarySyncStore>>,
    pub db: sea_orm::DatabaseConnection,
    pub fsio: Arc<fsio::FsIo>,
    pub root: std::path::PathBuf,
    pub node_id: uuid::Uuid,
    pub upload_lock: Arc<tokio::sync::Mutex<()>>,
    pub notify: Arc<dyn Fn() + Send + Sync>,
}

pub fn blob_router(context: BlobContext) -> Router {
    use axum::routing::put;
    Router::new()
        .route("/sync/artworks/{id}", get(get_artwork))
        .route(
            "/sync/blobs/{digest}",
            get(get_blob).head(head_blob).merge(put(put_blob)),
        )
        .with_state(context)
}

async fn get_artwork(
    State(context): State<BlobContext>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<axum::response::Response, StatusCode> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let store = context.store.read().await;
    let record = store
        .get(&ObjectKey {
            collection: Collection::Artworks,
            id,
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(Payload::Artwork(artwork)) = record.value else {
        return Err(StatusCode::NOT_FOUND);
    };
    let body = artwork
        .body()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let rows = entities::media_cover_art::Entity::find()
        .filter(entities::media_cover_art::Column::HlcUuid.eq(id.to_string()))
        .all(&context.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if rows.len() != 1 {
        return Err(StatusCode::NOT_FOUND);
    }
    let bytes = rows.into_iter().next().unwrap().binary;
    database::actions::cover_art::verify_body(id, &body.digest, body.size, &bytes)
        .map_err(|_| StatusCode::NOT_FOUND)?;
    axum::http::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_LENGTH, bytes.len())
        .body(axum::body::Body::from(bytes))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(serde::Deserialize)]
struct HeadQuery {
    track_id: uuid::Uuid,
}
async fn head_blob(
    State(context): State<BlobContext>,
    axum::extract::Path(digest): axum::extract::Path<String>,
    axum::extract::Query(query): axum::extract::Query<HeadQuery>,
) -> Result<StatusCode, StatusCode> {
    file::validate_digest(&digest).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = context.store.read().await;
    let record = store
        .get(&ObjectKey {
            collection: Collection::Tracks,
            id: query.track_id,
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(Payload::Track(track)) = record.value else {
        return Err(StatusCode::NOT_FOUND);
    };
    let blob = track
        .blob()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter(|blob| blob.digest == digest)
        .ok_or(StatusCode::NOT_FOUND)?;
    if file::local_audio_path(
        &context.db,
        &context.fsio,
        &context.root,
        query.track_id,
        &blob,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .is_some()
    {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn get_blob(
    State(context): State<BlobContext>,
    axum::extract::Path(digest): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    use std::io::{Read, Seek, SeekFrom};
    let path = file::cached_blob_path(&context.db, &context.fsio, &context.root, &digest)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut file = context
        .fsio
        .open(&path, "r")
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let size = file
        .seek(SeekFrom::End(0))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let requested = headers
        .get(axum::http::header::RANGE)
        .map(|h| h.to_str().ok().and_then(|r| parse_range(r, size)));
    let (start, length, partial) = match requested {
        None => (0, size, false),
        Some(Some((start, end))) => (start, end - start + 1, true),
        Some(None) => {
            return axum::http::Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(axum::http::header::CONTENT_RANGE, format!("bytes */{size}"))
                .header(axum::http::header::ACCEPT_RANGES, "bytes")
                .body(axum::body::Body::empty())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    file.seek(SeekFrom::Start(start))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let chunks = futures::stream::try_unfold((file, length), |(mut file, remaining)| async move {
        if remaining == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        let mut buffer = vec![0; remaining.min(64 * 1024) as usize];
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "audio file shortened",
            ));
        }
        buffer.truncate(count);
        Ok(Some((buffer, (file, remaining - count as u64))))
    });
    let mut response = axum::http::Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(axum::http::header::CONTENT_LENGTH, length)
        .header(axum::http::header::ACCEPT_RANGES, "bytes");
    if partial {
        response = response.header(
            axum::http::header::CONTENT_RANGE,
            format!("bytes {start}-{}/{size}", start + length - 1),
        );
    }
    response
        .body(axum::body::Body::from_stream(chunks))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let range = value.strip_prefix("bytes=")?;
    let (first, last) = range.split_once('-')?;
    if first.is_empty() {
        let suffix: u64 = last.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some((size.saturating_sub(suffix), size - 1));
    }
    let start: u64 = first.parse().ok()?;
    if start >= size {
        return None;
    }
    let end = if last.is_empty() {
        size - 1
    } else {
        last.parse::<u64>().ok()?.min(size - 1)
    };
    if end < start {
        return None;
    }
    Some((start, end))
}

struct UploadTemp {
    fsio: Arc<fsio::FsIo>,
    path: std::path::PathBuf,
}
impl Drop for UploadTemp {
    fn drop(&mut self) {
        let fsio = self.fsio.clone();
        let path = self.path.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = fsio.remove_file(&path).await;
            });
        }
    }
}

#[derive(serde::Deserialize)]
struct UploadQuery {
    track_id: uuid::Uuid,
    size: u64,
    extension: String,
}
async fn put_blob(
    State(context): State<BlobContext>,
    axum::extract::Path(digest): axum::extract::Path<String>,
    axum::extract::Query(query): axum::extract::Query<UploadQuery>,
    body: axum::body::Body,
) -> Result<StatusCode, StatusCode> {
    use futures::StreamExt;
    use std::io::Write;

    file::validate_digest(&digest).map_err(|_| StatusCode::BAD_REQUEST)?;
    file::validate_extension(&query.extension).map_err(|_| StatusCode::BAD_REQUEST)?;
    if query.size > file::MAX_BLOB_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let blob = BlobRef {
        digest: digest.clone(),
        size: query.size,
    };
    let key = ObjectKey {
        collection: Collection::Tracks,
        id: query.track_id,
    };
    let record = context
        .store
        .read()
        .await
        .get(&key)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if !matches!(record.value, Some(Payload::Track(ref track)) if track.blob().ok().flatten().as_ref() == Some(&blob) && track.extension == query.extension)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = downloads::prepare_download(
        &context.db,
        &context.fsio,
        &context.root,
        query.track_id,
        &blob,
        &query.extension,
    )
    .await
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    let temp = path.with_extension(format!("part-{}", uuid::Uuid::new_v4()));
    let _cleanup = UploadTemp {
        fsio: context.fsio.clone(),
        path: temp.clone(),
    };

    async {
        let mut file = context
            .fsio
            .open(&temp, "wt")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut stream = body.into_data_stream();
        let mut count = 0u64;
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(30), stream.next())
            .await
            .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
        {
            let chunk = chunk.map_err(|_| StatusCode::BAD_REQUEST)?;
            count = count
                .checked_add(chunk.len() as u64)
                .ok_or(StatusCode::PAYLOAD_TOO_LARGE)?;
            if count > query.size {
                return Err(StatusCode::PAYLOAD_TOO_LARGE);
            }
            file.write_all(&chunk)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        file.flush()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        drop(file);
        if count != query.size
            || file::hash_file(&context.fsio, &temp).map_err(|_| StatusCode::BAD_REQUEST)? != blob
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let _upload_guard = context.upload_lock.lock().await;
        if context
            .fsio
            .exists(&path)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            #[cfg(not(target_os = "android"))]
            {
                let metadata = std::fs::symlink_metadata(&path)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(StatusCode::BAD_REQUEST);
                }
            }
            if file::hash_file(&context.fsio, &path).ok().as_ref() != Some(&blob) {
                context
                    .fsio
                    .remove_file(&path)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            }
        }
        if !context
            .fsio
            .exists(&path)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            context
                .fsio
                .rename_file(&temp, &path)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        file::attach_download(
            &context.db,
            &context.store.read().await.record_store(),
            &context.fsio,
            &context.root,
            query.track_id,
            &blob,
            &path,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        (context.notify)();
        Ok(StatusCode::CREATED)
    }
    .await
}
