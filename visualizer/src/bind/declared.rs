//! A bind as it was written, in words: the text a lab shows of a `when` table and a `fallback`,
//! and the text it reads back (both are TOML values).

use toml::Value;

use crate::scene::FallbackDecl;

/// A TOML value in its inline form: `0.5`, `"out"`, `[8, 4, 2]`, `{ strength = 0.5 }`. A float
/// keeps its point, so that it reads back as a float.
pub fn inline(value: &Value) -> String {
    match value {
        Value::String(text) => format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::Integer(v) => v.to_string(),
        Value::Float(v) => format!("{v:?}"),
        Value::Boolean(v) => v.to_string(),
        Value::Datetime(v) => v.to_string(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(inline).collect::<Vec<_>>().join(", ")
        ),
        Value::Table(table) => {
            if table.is_empty() {
                "{}".to_string()
            } else {
                format!(
                    "{{ {} }}",
                    table
                        .iter()
                        .map(|(key, value)| format!("{key} = {}", inline(value)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
    }
}

/// The `when` of a bind, as an inline table.
pub fn when_text(when: &toml::Table) -> String {
    inline(&Value::Table(when.clone()))
}

/// The `fallback` of a bind as written: `"hits"`, `"rest"`, `{ value = 2.0 }`; empty for none.
pub fn fallback_text(fallback: Option<&FallbackDecl>) -> String {
    match fallback {
        None => String::new(),
        Some(FallbackDecl::Word(word)) => inline(&Value::String(word.clone())),
        Some(FallbackDecl::Value { value }) => format!("{{ value = {:?} }}", short(*value)),
    }
}

/// An f32 as the f64 with the same shortest text (`0.1`, not `0.10000000149011612`).
pub fn short(value: f32) -> f64 {
    format!("{value}").parse().unwrap_or(f64::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_written_inline_and_read_back() {
        let table: toml::Table =
            toml::from_str("every = [8, 4, 2]\ncurve = \"out\"\nsmooth = 1.0\nscale = true")
                .unwrap();
        let text = when_text(&table);
        assert_eq!(
            text,
            "{ curve = \"out\", every = [8, 4, 2], scale = true, smooth = 1.0 }"
        );
        let back: toml::Table = toml::from_str(&format!("when = {text}")).unwrap();
        assert_eq!(back["when"], toml::Value::Table(table));
        assert_eq!(when_text(&toml::Table::new()), "{}");
    }

    #[test]
    fn a_fallback_reads_as_written() {
        assert_eq!(fallback_text(None), "");
        assert_eq!(
            fallback_text(Some(&FallbackDecl::Word("hits".into()))),
            "\"hits\""
        );
        assert_eq!(
            fallback_text(Some(&FallbackDecl::Value { value: 2.0 })),
            "{ value = 2.0 }"
        );
        assert_eq!(
            fallback_text(Some(&FallbackDecl::Value { value: 0.1 })),
            "{ value = 0.1 }"
        );
    }
}
