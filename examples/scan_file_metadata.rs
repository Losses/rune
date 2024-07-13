use metadata::scanner::MetadataScanner;
use metadata::describe::describe_file;
use std::path::PathBuf;

fn to_unix_path_string(path_buf: PathBuf) -> Option<String> {
    let path = path_buf.as_path();

    path.to_str().map(|path_str| path_str.replace("\\", "/"))
}

#[tokio::main]
async fn main() {
    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let root_path = PathBuf::from(&path);

    let mut scanner = MetadataScanner::new(&path);

    // Example usage: Read 5 audio files at a time until no more files are available.
    while !scanner.has_ended() {
        let files = scanner.read_metadata(5);
        for file in files {
            let description = describe_file(&file.path, &root_path).unwrap();
            println!("= {}", to_unix_path_string(file.path).unwrap());
            println!("|- Description");
            println!("|  |- Hash: {}", description.file_hash);
            println!("|  |- Last Modified: {}", description.last_modified);
            println!("|- Metadata");
            for (key, value) in file.metadata {
                println!("|  |- {}: {}", key, value);
            }
        }
    }
}
