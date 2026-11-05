//! Utilities for applications that need to instantiate different types of stream handlers
//! depending on the type of input supplied.
//!
//! A [Registry] can route inputs to different handlers depending on a set of rules.
//!
//! ```no_run
//! use std::error::Error;
//! use std::num::NonZeroUsize;
//!
//! use async_trait::async_trait;
//! use stream_download::registry::{Input, Registry, RegistryEntry, Rule};
//! use stream_download::source::DecodeError;
//! use stream_download::storage::adaptive::AdaptiveStorageProvider;
//! use stream_download::storage::temp::TempStorageProvider;
//! use stream_download::{Settings, StreamDownload};
//!
//! type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
//! type Downloader =
//!     StreamDownload<AdaptiveStorageProvider<TempStorageProvider, TempStorageProvider>>;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let mut registry = Registry::new().entry(HttpResolver::new());
//!
//!     let reader = registry
//!         .find_match("http://some-site.com/some-url")
//!         .await
//!         .ok_or_else(|| format!("no entries matched"))??;
//!     Ok(())
//! }
//!
//! pub struct HttpResolver {
//!     rules: Vec<Rule>,
//! }
//!
//! impl HttpResolver {
//!     pub fn new() -> Self {
//!         Self {
//!             rules: vec![Rule::any_http()],
//!         }
//!     }
//! }
//!
//! #[async_trait]
//! impl RegistryEntry<Result<Downloader>> for HttpResolver {
//!     fn priority(&self) -> u32 {
//!         1
//!     }
//!
//!     fn rules(&self) -> &[Rule] {
//!         &self.rules
//!     }
//!
//!     async fn handler(&mut self, input: Input) -> Result<Downloader> {
//!         let settings = Settings::default();
//!         let reader = match StreamDownload::new_http(
//!             input.source.into_url(),
//!             // use adaptive storage to keep the underlying size bounded when the stream has no
//!             // content length
//!             AdaptiveStorageProvider::new(
//!                 TempStorageProvider::default(),
//!                 // ensure we have enough buffer space to store the prefetch data
//!                 NonZeroUsize::new((settings.get_prefetch_bytes() * 2) as usize).unwrap(),
//!             ),
//!             settings,
//!         )
//!         .await
//!         {
//!             Ok(reader) => reader,
//!             Err(e) => return Err(e.decode_error().await)?,
//!         };
//!         Ok(reader)
//!     }
//! }
//! ```

mod matcher;

use std::fmt;

use educe::Educe;
pub use matcher::*;
use url::Url;

/// An entry that can be chosen by a [`Registry`] if matched by a given [`Rule`].
#[async_trait::async_trait]
pub trait RegistryEntry<T> {
    /// Defines the priority that this entry will be evaluated relative to the other entries. Lower
    /// numbers have precedence over higher ones.
    fn priority(&self) -> u32;
    /// The list of rules that will cause the entry to be chosen.
    fn rules(&self) -> &[Rule];
    /// Handler to run when this entry is selected.
    async fn handler(&mut self, input: Input) -> T;
}

/// The input source. Can be a URL or any generic string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// URL input.
    Url(Url),
    /// Generic string input.
    String(String),
}

impl From<String> for Source {
    fn from(value: String) -> Self {
        match Url::parse(&value) {
            Ok(url) => Self::Url(url),
            Err(_) => Self::String(value),
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(url) => url.fmt(f),
            Self::String(s) => s.fmt(f),
        }
    }
}

impl From<Source> for String {
    fn from(value: Source) -> Self {
        match value {
            Source::Url(url) => url.to_string(),
            Source::String(s) => s.to_string(),
        }
    }
}

impl Source {
    /// Extracts the inner [Url] from this source if available.
    pub fn try_into_url(self) -> Option<Url> {
        match self {
            Self::Url(url) => Some(url),
            Self::String(_) => None,
        }
    }

    /// Extracts the inner [Url] from this source, panicking if not available.
    ///
    /// # Panics
    ///
    /// If the type is not [`Source::Url`].
    pub fn into_url(self) -> Url {
        self.try_into_url().expect("incorrect input type")
    }

    /// Extracts the inner [String] from this source if available.
    pub fn try_into_string(self) -> Option<String> {
        match self {
            Self::String(s) => Some(s),
            Self::Url(_) => None,
        }
    }

    /// Extracts the inner [String] from this source, panicking if not available.
    ///
    /// # Panics
    ///
    /// If the type is not [`Source::String`].
    pub fn into_string(self) -> String {
        self.try_into_string().expect("incorrect input type")
    }
}

/// Input supplied to the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// Prefix parsed from a [`Rule::Prefix`], if applicable.
    pub prefix: Option<String>,
    /// Input source.
    pub source: Source,
}

impl Input {
    /// Converts the input back into its raw string format.
    pub fn into_raw(self) -> String {
        format!("{self}")
    }
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = &self.prefix {
            prefix.fmt(f)?;
        }
        self.source.fmt(f)
    }
}

/// Input to pass to a [`Registry`].
#[derive(Clone, Debug)]
pub enum InputType {
    /// Raw string input.
    Raw(String),
    /// Input that has already been parsed.
    Parsed(Input),
}

impl From<Input> for InputType {
    fn from(value: Input) -> Self {
        Self::Parsed(value)
    }
}

impl<T> From<T> for InputType
where
    T: Into<String>,
{
    fn from(value: T) -> Self {
        Self::Raw(value.into())
    }
}

/// A struct that manages a collection of [registry entries][RegistryEntry].
#[derive(Educe)]
#[educe(Debug)]
pub struct Registry<T> {
    #[educe(Debug = false)]
    generators: Vec<Box<dyn RegistryEntry<T> + Send + Sync>>,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! filter_matches {
    ($iter:expr, $v:ident, $enum_match:pat) => {
        $iter
            .iter()
            .filter_map(|m| if let $enum_match = m { Some($v) } else { None })
    };
}

impl<T> Registry<T> {
    /// Creates a new [`Registry`] with no entries.
    pub fn new() -> Self {
        Self {
            generators: Vec::new(),
        }
    }

    /// Adds a [`RegistryEntry`] to the registry.
    pub fn entry<E>(mut self, entry: E) -> Self
    where
        E: RegistryEntry<T> + Send + Sync + 'static,
    {
        self.generators.push(Box::new(entry));
        self.generators.sort_by_key(|k| k.priority());
        self
    }

    /// Checks the given input against the registry and returns the first matching value, if found.
    pub async fn find_match<I>(&mut self, input: I) -> Option<T>
    where
        I: Into<InputType>,
    {
        let input = input.into();
        let parsed_input = match &input {
            InputType::Raw(raw_input) => Input {
                prefix: None,
                source: raw_input.clone().into(),
            },
            InputType::Parsed(parsed_input) => parsed_input.clone(),
        };
        for generator in &mut self.generators {
            let prefix_rules = filter_matches!(generator.rules(), m, Rule::Prefix(m));
            match &input {
                InputType::Raw(raw_input) => {
                    for rule in prefix_rules {
                        if let Some(input) = rule.match_input(raw_input) {
                            let input = Input {
                                prefix: Some(rule.prefix.clone()),
                                source: input.into(),
                            };
                            return Some(generator.handler(input).await);
                        }
                    }
                }
                InputType::Parsed(parsed_input) => {
                    for rule in prefix_rules {
                        if rule.matches_parsed(parsed_input) {
                            return Some(generator.handler(parsed_input.clone()).await);
                        }
                    }
                }
            };
            if let res @ Some(_) = handle(generator, &parsed_input).await {
                return res;
            }
        }

        None
    }
}

async fn handle<T>(
    generator: &mut Box<dyn RegistryEntry<T> + Send + Sync>,
    parsed_input: &Input,
) -> Option<T> {
    let matchers = generator.rules();
    match &parsed_input.source {
        Source::Url(url) => {
            let url_rules = filter_matches!(matchers, m, Rule::Url(m));
            for rule in url_rules {
                if rule.matches_input(url) {
                    return Some(generator.handler(parsed_input.clone()).await);
                }
            }
        }
        Source::String(id) => {
            let string_rules = filter_matches!(matchers, m, Rule::String(m));
            for rule in string_rules {
                if rule.matches_input(id) {
                    return Some(generator.handler(parsed_input.clone()).await);
                }
            }
        }
    }
    None
}
