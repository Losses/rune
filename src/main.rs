use database::connection::connect_main_db;

#[tokio::main]
async fn main() {
    let path = ".";
    let db = connect_main_db(path).await;

    println!("{:#?}", db);
}
