#[test]
fn rustls_has_an_unambiguous_default_provider() {
    let _builder = rustls::ClientConfig::builder();
}
