fn main() {
    if pkg_config::Config::new().probe("lmdb").is_err() {
        panic!("Could not find liblmdb using pkg-config");
    }
}
