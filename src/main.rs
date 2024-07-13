use database::connection::connect_main_db;
use metadata::reader::get_metadata;

#[tokio::main]
async fn main() {
    let path = ".";
    let db = connect_main_db(path).await;

    println!("{:#?}", db);

    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("file path not provided");

    match get_metadata(path, None) {
        Ok(metadata) => {
            for (key, value) in metadata {
                println!("{}: {}", key, value);
            }
        }
        Err(err) => eprintln!("Error: {}", err),
    }
}
