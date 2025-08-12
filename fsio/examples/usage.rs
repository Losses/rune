use std::path::Path;

use fsio::FsIo;

#[tokio::main]
async fn main() {
    // Non-Android Usage
    #[cfg(not(target_os = "android"))]
    {
        println!("Running standard FS I/O example...");
        let fs = FsIo::new();
        let base_path = Path::new("/tmp/fsio_example");

        // Cleanup previous runs
        if fs.exists(base_path).unwrap_or(false) {
            fs.remove_dir_all(base_path)
                .await
                .expect("Failed to clean up temp dir");
        }

        // 1. Create a directory
        fs.create_dir_all(base_path)
            .expect("Failed to create directory");
        println!("Created directory: {base_path:?}");

        // 2. Write a file
        let file_path = base_path.join("test.txt");
        let content = "Hello, fsio!";
        fs.write(&file_path, content.as_bytes())
            .await
            .expect("Failed to write to file");
        println!("Wrote to file: {file_path:?}");

        // 3. Read the file
        let read_content = fs.read(&file_path).expect("Failed to read file");
        assert_eq!(content.as_bytes(), read_content.as_slice());
        println!("Read and verified content from file.");

        // 4. List directory contents
        let nodes = fs
            .read_dir(base_path)
            .await
            .expect("Failed to read directory");
        println!("Directory contents: {nodes:#?}");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].path, file_path);

        // 5. Clean up
        fs.remove_dir_all(base_path)
            .await
            .expect("Failed to remove directory");
        println!("Cleaned up directory: {base_path:?}");
    }

    // Android-Specific Usage (Conceptual)
    #[cfg(target_os = "android")]
    {
        println!("This is a conceptual example for Android.");
        // On Android, you would get the root URI from the system's file picker
        // (e.g., ACTION_OPEN_DOCUMENT_TREE).
        // The database path would be a path in your app's private data directory.

        // let root_uri = "content://com.android.externalstorage.documents/tree/primary%3AMusic";
        // let db_path = Path::new("/data/data/com.yourapp/files/fs_cache.db");

        // let fs = FsIo::new(db_path, root_uri).await.expect("Failed to init Android FsIo");

        // You can now use `fs` in the same way as the standard version.
        // let music_dir = Path::new("MyFavoriteBand");
        // if !fs.exists(music_dir).await.unwrap() {
        //     fs.create_dir(Path::new(""), "MyFavoriteBand").await.expect("Failed to create dir on SAF");
        // }
        // println!("Directory operations would work similarly.");
    }

    println!("Example finished.");
}
