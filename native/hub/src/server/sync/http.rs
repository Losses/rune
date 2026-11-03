//! Audio content endpoints mounted by an authenticated caller.

use anyhow::Result;
use axum::body::Body;
use axum::http::Response;
use axum::{
    Router,
    extract::State,
    http::{
        StatusCode,
        header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, RANGE},
    },
    routing::get,
};
use database::{
    actions::{cover_art::verify_body, file},
    entities,
    sync::{
        LibrarySyncStore,
        model::{Collection, ObjectKey, Payload},
    },
};
use entities::media_cover_art::Column;
use entities::media_cover_art::Entity;
use std::io::Error;
use std::io::ErrorKind;
use std::sync::Arc;

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
    Router::new()
        .route("/sync/artworks/{id}", get(get_artwork))
        .route("/sync/blobs/{digest}", get(get_blob).head(head_blob))
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
    let rows = Entity::find()
        .filter(Column::HlcUuid.eq(id.to_string()))
        .all(&context.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if rows.len() != 1 {
        return Err(StatusCode::NOT_FOUND);
    }
    let bytes = rows.into_iter().next().unwrap().binary;
    verify_body(id, &body.digest, body.size, &bytes).map_err(|_| StatusCode::NOT_FOUND)?;
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_LENGTH, bytes.len())
        .body(Body::from(bytes))
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
        .get(RANGE)
        .map(|h| h.to_str().ok().and_then(|r| parse_range(r, size)));
    let (start, length, partial) = match requested {
        None => (0, size, false),
        Some(Some((start, end))) => (start, end - start + 1, true),
        Some(None) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(CONTENT_RANGE, format!("bytes */{size}"))
                .header(ACCEPT_RANGES, "bytes")
                .body(Body::empty())
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
            return Err(Error::new(ErrorKind::UnexpectedEof, "audio file shortened"));
        }
        buffer.truncate(count);
        Ok(Some((buffer, (file, remaining - count as u64))))
    });
    let mut response = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(CONTENT_LENGTH, length)
        .header(ACCEPT_RANGES, "bytes");
    if partial {
        response = response.header(
            CONTENT_RANGE,
            format!("bytes {start}-{}/{size}", start + length - 1),
        );
    }
    response
        .body(Body::from_stream(chunks))
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
