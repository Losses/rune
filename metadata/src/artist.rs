// Historical separators: [", ", "; ", " × ", " x ", " / ", " ft.", " ft. ", " feat. ", " & "]
pub(crate) const DEFAULT_SEPARATORS: &[&str] = &[";"];

/// Split an artist value on semicolons.
pub fn split_artists(input: &str) -> Vec<String> {
    split_artists_with_separators(input, DEFAULT_SEPARATORS)
}

/// Split an artist value using literal separators supplied by the caller.
pub fn split_artists_with_separators(input: &str, separators: &[&str]) -> Vec<String> {
    crate::delimiters::split(input, separators)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}
