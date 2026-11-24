pub fn split_genres(x: &str) -> Vec<String> {
    x.split("\\\\").map(|s| s.to_string()).collect()
}
