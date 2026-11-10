use tokio::net::TcpListener;
use url::Url;

pub struct TestServer {
    pub url: Url,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn serve(app: axum::Router) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    TestServer {
        url: Url::parse(&format!("http://{address}")).unwrap(),
        task,
    }
}
