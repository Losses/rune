use std::path::PathBuf;

use ::fsio::FsIo;
use ::metadata::{
    describe::{FileDescription, describe_file},
    scanner::AudioScanner,
};

fn to_unix_path_string(path_buf: PathBuf) -> Option<String> {
    let path = path_buf.as_path();

    path.to_str().map(|path_str| path_str.replace("\\", "/"))
}

#[tokio::main]
async fn main() {
    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");
    let fsio = FsIo::new();

    let root_path = PathBuf::from(&path);

    let mut scanner = AudioScanner::new(&fsio, &path).unwrap();

    // Example usage: Read 5 audio files at a time until no more files are available.
    while !scanner.has_ended() {
        let files = scanner.read_files(5);

        let descriptions: Vec<Option<FileDescription>> = files
            .clone()
            .into_iter()
            .map(|fs_node| describe_file(&fs_node, &Some(root_path.to_path_buf())))
            .map(|result| result.ok())
            .collect();

        for description in descriptions {
            let mut d = description.unwrap();

            println!("= {}", to_unix_path_string(d.actual_path.clone()).unwrap());
            println!("|- Description");
            println!("|  |- Hash: {}", d.get_crc(&fsio).unwrap());
            println!("|  |- Last Modified: {}", d.last_modified);
            println!("|- Metadata");
        }
    }
}
