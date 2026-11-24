use std::mem;

use anyhow::{Context, Error, Result};
use log::{error, info};
use sea_orm::{DatabaseConnection, TransactionTrait, prelude::*};
use tokio_util::sync::CancellationToken;

use crate::entities::media_files;

use super::resolved_metadata::portable::publish_track;

/// Indexes media files by processing their metadata and updating database records for artists, albums, and genres.
///
/// This function processes a list of media file IDs, retrieves metadata summaries for each file,
/// and then iterates through each summary to update artist, album, and genre information in the database.
/// Each file is processed within its own database transaction to ensure atomicity.
/// It also supports cancellation via a `CancellationToken`.
///
/// # Arguments
///
/// * `main_db`: A reference to the database connection.
/// * `file_ids`: A vector of media file IDs to index.
/// * `cancel_token`: An optional cancellation token to stop the operation prematurely.
///
/// # Returns
///
/// Returns `Ok(())` if the indexing process is successful, or an `Err(Error)` if any error occurs.
pub async fn index_media_files(
    main_db: &DatabaseConnection,
    _node_id: &str,
    file_ids: Vec<i32>,
    cancel_token: Option<&CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    info!("Indexing media: {file_ids:?}");

    // Check for cancellation before starting any processing.
    if let Some(token) = &cancel_token
        && token.is_cancelled()
    {
        info!("Operation cancelled before starting.");
        return Ok(());
    }

    for file_id in file_ids {
        // Start a new transaction for each file to ensure individual processing atomicity.
        let txn = match main_db.begin().await {
            Ok(txn) => txn,
            Err(e) => {
                error!("Failed to start transaction: {e}");
                continue; // Proceed to the next file if transaction start fails.
            }
        };

        // Check for cancellation during processing of each file.
        if let Some(token) = &cancel_token
            && token.is_cancelled()
        {
            info!("Operation cancelled during processing.");
            txn.rollback().await?;
            return Ok(());
        }

        // Derive relationships from the current source inside the writer transaction.
        store.lock(&txn).await?;
        match publish_track(&txn, file_id, cancel_token, store, None).await {
            Ok(true) => txn.commit().await?,
            Ok(false) => txn.rollback().await?,
            Err(e) => {
                txn.rollback().await.with_context(|| {
                    format!("Failed to roll back indexing file {file_id} after {e:#}")
                })?;
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Indexes the entire audio library in batches, processing media files in parallel.
///
/// This function fetches media files in batches from the database and processes them
/// using `index_media_files`. It utilizes an asynchronous channel to pass files from
/// a producer task (fetching files) to a consumer task (indexing files), allowing for
/// concurrent processing. After indexing is complete, it performs library maintenance.
///
/// # Arguments
///
/// * `main_db`: A reference to the database connection.
/// * `batch_size`: The number of media files to process in each batch.
/// * `cancel_token`: An optional cancellation token to stop the operation prematurely.
///
/// # Returns
///
/// Returns `Ok(())` if the library indexing is successful, or an `Err(Error)` if any error occurs.
pub async fn index_audio_library(
    main_db: &DatabaseConnection,
    node_id: &str,
    batch_size: usize,
    cancel_token: Option<&CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    anyhow::ensure!(batch_size > 0, "index batch size must be positive");
    let mut cursor = media_files::Entity::find().cursor_by(media_files::Column::Id); // Create a cursor for batch fetching.
    let (tx, rx) = async_channel::bounded(batch_size); // Create an async channel for file batching.

    // Producer task: Fetches media files in batches from the database and sends them to the channel.
    let producer = async {
        loop {
            // Fetch the next batch of files from the database using the cursor.
            let files: Vec<media_files::Model> =
                cursor.first(batch_size.try_into()?).all(main_db).await?;

            if files.is_empty() {
                info!("No more files to process. Exiting loop.");
                break; // Exit loop if no more files are found.
            }

            // Send each file to the consumer via the channel.
            for file in &files {
                tx.send(file.clone()).await?;
            }

            // Move the cursor to the next batch based on the last file's ID.
            if let Some(last_file) = files.last() {
                info!("Moving cursor after file ID: {}", last_file.id);
                cursor.after(last_file.id);
            } else {
                break; // Exit loop if there was an issue getting the last file.
            }
        }

        drop(tx); // Close the channel to signal consumers to stop after all files are sent.
        Ok::<(), Error>(())
    };

    // Consumer task: Receives media files from the channel and processes them in batches.
    let consumer = async move {
        let mut file_ids = Vec::with_capacity(batch_size); // Initialize vector to hold file IDs for batch processing.

        // Receive files from the channel until the channel is closed.
        while let Ok(file) = rx.recv().await {
            file_ids.push(file.id); // Add file ID to the current batch.

            // Process the batch when it reaches the specified batch size.
            if file_ids.len() >= batch_size {
                process_batch(main_db, node_id, &mut file_ids, cancel_token, store).await?; // Process the current batch.
            }
        }

        // Process any remaining files in the last batch after the channel is closed.
        if !file_ids.is_empty() {
            process_batch(main_db, node_id, &mut file_ids, cancel_token, store).await?; // Process the last batch.
        }

        Ok::<(), Error>(())
    };

    // Run the producer and consumer tasks concurrently.
    let (producer_result, consumer_result) = futures::join!(producer, consumer);
    producer_result?; // Propagate errors from producer task.
    consumer_result?; // Propagate errors from consumer task.

    info!("Audio indexing analysis completed.");

    Ok(())
}

/// Processes a batch of file IDs by calling `index_media_files`.
///
/// This is a helper function to handle batch processing of media files. It takes a mutable
/// vector of file IDs, processes them using `index_media_files`, and clears the vector.
///
/// # Arguments
///
/// * `db`: A reference to the database connection.
/// * `file_ids`: A mutable vector of file IDs to process.
/// * `cancel_token`: An optional cancellation token to stop the operation prematurely.
///
/// # Returns
///
/// Returns `Ok(())` if batch processing is successful, or an `Err(Error)` if any error occurs.
async fn process_batch(
    db: &DatabaseConnection,
    node_id: &str,
    file_ids: &mut Vec<i32>,
    cancel_token: Option<&CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    let batch = mem::take(file_ids);
    index_media_files(db, node_id, batch, cancel_token, store).await
}
