use database::{
    actions::file::{file_ids_by_uuid, get_file_by_id, get_random_files, materialized_path},
    connection::MainDbConnection,
    entities::media_files,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{Result, bail};
use dunce::canonicalize;
use futures::future::{BoxFuture, join_all};
use log::{debug, error, info};
use tokio::task;
use uuid::Uuid;

use fsio::FsIo;
use playback::{
    player::{Playable, PlaybackSourceResolver, Player, PlayingItem, RemoteAudioSource},
    strategies::AddMode,
};

struct LocalSourceResolver {
    db: MainDbConnection,
    fsio: Arc<fsio::FsIo>,
    root: PathBuf,
}

impl PlaybackSourceResolver for LocalSourceResolver {
    fn resolve_id(&self, file_uuid: Uuid) -> BoxFuture<'static, Result<Option<i32>>> {
        let db = self.db.clone();
        Box::pin(async move {
            let ids = file_ids_by_uuid(&db, &[file_uuid]).await?;
            Ok(ids.get(&file_uuid).copied())
        })
    }

    fn local_path(&self, id: i32) -> BoxFuture<'static, Result<Option<PathBuf>>> {
        let db = self.db.clone();
        let fsio = self.fsio.clone();
        let root = self.root.clone();
        Box::pin(async move { materialized_path(&db, &fsio, &root, id).await })
    }

    fn resolve(&self, _id: i32) -> BoxFuture<'static, Result<RemoteAudioSource>> {
        Box::pin(async { bail!("Local CLI playback requires a local audio file") })
    }
}

async fn play_files(
    main_db: &MainDbConnection,
    canonicalized_path: &Path,
    file_ids: Vec<i32>,
    library_id: &str,
) {
    let fsio = Arc::new(FsIo::new());
    let player = Player::new(fsio.clone(), None);
    player.set_source_resolver(Arc::new(LocalSourceResolver {
        db: main_db.clone(),
        fsio,
        root: canonicalized_path.to_path_buf(),
    }));
    let player = Arc::new(Mutex::new(player));

    let file_futures = file_ids.into_iter().map(|id| async move {
        match get_file_by_id(main_db, id).await {
            Ok(file) => Some(file),
            Err(e) => {
                error!("Failed to get file by id {id}: {e}");
                None
            }
        }
    });

    let files: Vec<media_files::Model> = join_all(file_futures)
        .await
        .into_iter()
        .filter_map(|file| file.flatten())
        .collect();

    let library_uuid = match Uuid::parse_str(library_id) {
        Ok(library_uuid) => library_uuid,
        Err(error) => {
            error!("Invalid library id {library_id}: {error}");
            return;
        }
    };

    player.lock().unwrap().add_to_playlist(
        files
            .into_iter()
            .filter_map(|file| {
                if let Err(error) = Uuid::parse_str(&file.source_uuid) {
                    error!("Invalid file UUID {}: {error}", file.source_uuid);
                    return None;
                }
                let path = canonicalized_path.join(file.directory).join(file.file_name);
                let path = match canonicalize(&path) {
                    Ok(path) => path,
                    Err(error) => {
                        error!(
                            "Failed to canonicalize file {} at {}: {error}",
                            file.id,
                            path.display()
                        );
                        return None;
                    }
                };
                Some((
                    PlayingItem::LibraryTrack {
                        library_id: library_uuid,
                        track: file.source_uuid,
                    },
                    path,
                ))
            })
            .collect(),
        AddMode::AppendToEnd,
    );

    player.lock().unwrap().play();

    let status_receiver = player.lock().unwrap().subscribe_status();

    info!("Initializing event listeners");
    task::spawn(async move {
        while let Ok(status) = status_receiver.recv().await {
            debug!("Player status updated: {status:?}");

            let position = status.position;

            debug!(
                "State: {}, seconds: {}",
                status.state,
                position.as_secs_f32()
            );
        }
    });

    thread::sleep(Duration::from_millis(30000));
}

pub async fn play_random(main_db: &MainDbConnection, canonicalized_path: &Path, library_id: &str) {
    match get_random_files(main_db, 30).await {
        Ok(files) => {
            let file_ids = files.into_iter().map(|file| file.id).collect();
            play_files(main_db, canonicalized_path, file_ids, library_id).await;
        }
        Err(e) => {
            error!("Failed to get random files: {e}");
        }
    }
}

pub async fn play_by_id(
    main_db: &MainDbConnection,
    canonicalized_path: &Path,
    id: i32,
    library_id: &str,
) {
    play_files(main_db, canonicalized_path, vec![id], library_id).await;
}
