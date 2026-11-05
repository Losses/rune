use regex::Regex;
use url::Url;

use super::Input;

#[derive(Clone, Debug)]
enum MatchType {
    Any,
    Value(Matcher),
}

impl MatchType {
    fn matches_input(&self, input: Option<&str>) -> bool {
        match self {
            Self::Any => true,
            Self::Value(val) => {
                if let Some(input) = input {
                    val.is_match(input)
                } else {
                    false
                }
            }
        }
    }
}

/// Matches strings using either a regex or simple equality check.
#[derive(Clone, Debug)]
pub enum Matcher {
    /// Regex matcher.
    Regex(Regex),
    /// Simple string matcher.
    String(String),
}

impl Matcher {
    fn is_match(&self, search: &str) -> bool {
        match self {
            Self::Regex(re) => re.is_match(search),
            Self::String(s) => s == search,
        }
    }
}

impl From<Regex> for Matcher {
    fn from(value: Regex) -> Self {
        Self::Regex(value)
    }
}

impl From<String> for Matcher {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for Matcher {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

/// A [Rule] is used to check if some input matches the given criteria.
#[derive(Clone, Debug)]
pub enum Rule {
    /// Checks the prefix of the given input.
    Prefix(PrefixMatcher),
    /// Checks raw string inputs.
    String(StringMatcher),
    /// Checks URL inputs.
    Url(UrlMatcher),
}

impl Rule {
    /// Looks for prefix substrings. This is mainly useful for attaching a custom protocol that can
    /// force a specific type of handler to be used.
    pub fn prefix<S>(prefix: S) -> Self
    where
        S: Into<String>,
    {
        Self::Prefix(PrefixMatcher {
            prefix: prefix.into(),
        })
    }

    /// Matches string (non-URL) inputs.
    pub fn string<M>(s: M) -> Self
    where
        M: Into<Matcher>,
    {
        Self::String(StringMatcher {
            match_type: MatchType::Value(s.into()),
        })
    }

    /// Matches any string (non-URL) input.
    pub fn any_string() -> Self {
        Self::String(StringMatcher {
            match_type: MatchType::Any,
        })
    }

    /// Matches any URL.
    pub fn any_url() -> Self {
        Self::Url(UrlMatcher {
            scheme: MatchType::Any,
            domain: MatchType::Any,
        })
    }

    /// Matches a specific URL scheme.
    pub fn url_scheme<M>(scheme: M) -> Self
    where
        M: Into<Matcher>,
    {
        Self::Url(UrlMatcher {
            scheme: MatchType::Value(scheme.into()),
            domain: MatchType::Any,
        })
    }

    /// Matches any HTTP or HTTPS URL.
    pub fn any_http() -> Self {
        Self::Url(UrlMatcher {
            scheme: MatchType::Value(http_regex().into()),
            domain: MatchType::Any,
        })
    }

    /// Matches a specific URL domain.
    pub fn url_domain<M>(domain: M) -> Self
    where
        M: Into<Matcher>,
    {
        Self::Url(UrlMatcher {
            scheme: MatchType::Any,
            domain: MatchType::Value(domain.into()),
        })
    }

    /// Matches a specific domain for any HTTP or HTTPS URL.
    pub fn http_domain<M>(domain: M) -> Self
    where
        M: Into<Matcher>,
    {
        Self::Url(UrlMatcher {
            scheme: MatchType::Value(http_regex().into()),
            domain: MatchType::Value(domain.into()),
        })
    }

    /// Matches a specific URL scheme and domain.
    pub fn url<M1, M2>(scheme: M1, domain: M2) -> Self
    where
        M1: Into<Matcher>,
        M2: Into<Matcher>,
    {
        Self::Url(UrlMatcher {
            scheme: MatchType::Value(scheme.into()),
            domain: MatchType::Value(domain.into()),
        })
    }
}

/// Helper method to create a regex that will match HTTP or HTTPS URL schemes.
#[expect(clippy::missing_panics_doc)]
pub fn http_regex() -> Regex {
    // Only panics if the regex is mistyped
    Regex::new(r"^https?$").expect("invalid regex")
}

/// Matches input prefixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixMatcher {
    pub(super) prefix: String,
}

impl PrefixMatcher {
    pub(super) fn match_input(&self, input: &str) -> Option<String> {
        if input.starts_with(&self.prefix) {
            Some(input.replacen(&self.prefix, "", 1))
        } else {
            None
        }
    }

    pub(super) fn matches_parsed(&self, input: &Input) -> bool {
        input.prefix.as_deref() == Some(&self.prefix)
    }
}

/// Matches raw (non-URL) strings.
#[derive(Debug, Clone)]
pub struct StringMatcher {
    match_type: MatchType,
}

/// Matches any (non-URL) string.
impl StringMatcher {
    pub(super) fn matches_input(&self, input: &str) -> bool {
        self.match_type.matches_input(Some(input))
    }
}

/// Matches any URL.
#[derive(Debug, Clone)]
pub struct UrlMatcher {
    scheme: MatchType,
    domain: MatchType,
}

impl UrlMatcher {
    pub(super) fn matches_input(&self, input: &Url) -> bool {
        self.scheme.matches_input(Some(input.scheme())) && self.domain.matches_input(input.domain())
    }
}
