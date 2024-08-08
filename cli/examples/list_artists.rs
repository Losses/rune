use database::actions::artists::count_artists_by_first_letter;
use database::connection::connect_main_db;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let db = connect_main_db(&path).await.unwrap();

    let _ = count_artists_by_first_letter(&db).await;

    println!("OK");
}
