use fsio::FsIo;
use metadata::reader::get_metadata;

fn main() {
    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("file path not provided");
    let fsio = FsIo::new();

    match get_metadata(&fsio.canonicalize_str(path).unwrap(), None) {
        Ok(metadata) => {
            for (key, value) in metadata {
                println!("{key}: {value}");
            }
        }
        Err(err) => eprintln!("Error: {err}"),
    }
}
