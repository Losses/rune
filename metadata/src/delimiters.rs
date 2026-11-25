/// Split on the earliest literal delimiter. List order breaks ties.
pub(crate) fn split<'a, 'b>(
    value: &'a str,
    delimiters: &'b [&'b str],
) -> impl Iterator<Item = &'a str> + use<'a, 'b> {
    let mut rest = Some(value);
    std::iter::from_fn(move || {
        let value = rest.take()?;
        let next = match delimiters {
            [] | [""] => None,
            [delimiter] => value.find(delimiter).map(|start| (start, delimiter.len())),
            _ => value.char_indices().find_map(|(start, _)| {
                delimiters
                    .iter()
                    .find(|delimiter| {
                        !delimiter.is_empty() && value[start..].starts_with(**delimiter)
                    })
                    .map(|delimiter| (start, delimiter.len()))
            }),
        };

        // Consume the separator so consecutive matches produce separate fields.
        if let Some((start, length)) = next {
            rest = Some(&value[start + length..]);
            Some(&value[..start])
        } else {
            Some(value)
        }
    })
}
