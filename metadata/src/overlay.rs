//! Local corrections applied to resolved metadata without changing source evidence.

use std::collections::BTreeMap;

use anyhow::{Result, ensure};

use crate::model::{
    AlbumMetadata, AudioMetadata, DiscMetadata, FieldSource, MetadataBase, MetadataProvenance,
    MetadataSourceKind, ReleaseTrackMetadata, ReleaseType,
};

macro_rules! overlay_fields {
    ($($variant:ident => $key:literal),+ $(,)?) => {
        /// Fields supported by the local metadata source.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub enum OverlayField { $($variant),+ }

        impl OverlayField {
            pub const fn key(self) -> &'static str {
                match self { $(Self::$variant => $key),+ }
            }

            pub fn from_key(key: &str) -> Option<Self> {
                match key { $($key => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

overlay_fields! {
    Title => "title", Version => "version", CatalogNumber => "catalog_number",
    ReleaseDate => "release_date", OriginalReleaseDate => "original_release_date",
    Label => "labels", ReleaseType => "release_type", Barcode => "barcode",
    ReleaseCountry => "release_country", Description => "description",
    Comment => "comment", SortTitle => "sort_title", Subtitle => "subtitle",
}

impl OverlayField {
    /// Validate and normalize a submitted value before storing it.
    pub fn normalize(self, value: &str) -> Result<String> {
        let value = value.trim();
        ensure!(
            !value.is_empty() && value.len() <= 16384 && !value.contains('\0'),
            "invalid metadata value"
        );
        if self == Self::ReleaseType {
            return ReleaseType::from_label(value)
                .map(|kind| kind.as_str().to_owned())
                .ok_or_else(|| anyhow::anyhow!("invalid release type"));
        }
        if matches!(self, Self::ReleaseDate | Self::OriginalReleaseDate) {
            let date = match value.len() {
                4 => format!("{value}-01-01"),
                7 => format!("{value}-01"),
                _ => value.to_owned(),
            };
            ensure!(
                date.len() == 10 && date.parse::<jiff::civil::Date>().is_ok(),
                "invalid release date"
            );
        }
        Ok(value.to_owned())
    }
}

/// Set a value, mask a lower-priority value, or remove the local correction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayEdit {
    Set(String),
    Clear,
    Restore,
}

/// Values asserted by a metadata source. A present null masks lower-priority sources.
#[derive(Debug, Clone, Default)]
pub struct MetadataOverlay {
    values: BTreeMap<OverlayField, Option<String>>,
}

impl MetadataOverlay {
    pub fn insert(&mut self, field: OverlayField, value: Option<String>) {
        self.values.insert(field, value);
    }

    pub fn apply_to<T: OverlayMetadata>(self, target: &mut T) {
        for (field, value) in self.values {
            target.apply_field(
                field,
                value,
                FieldSource {
                    kind: MetadataSourceKind::LocalOverlay,
                    key: Some(field.key().into()),
                    path: None,
                    pattern: None,
                    entity_id: None,
                    confidence: None,
                },
            );
        }
    }
}

/// A metadata entity with descriptive fields that an overlay can replace.
pub trait OverlayMetadata {
    fn apply_field(&mut self, field: OverlayField, value: Option<String>, source: FieldSource);
}

fn common(base: &mut MetadataBase, field: OverlayField) -> Option<&mut Option<String>> {
    match field {
        OverlayField::Version => Some(&mut base.version),
        OverlayField::ReleaseDate => Some(&mut base.release_date),
        OverlayField::OriginalReleaseDate => Some(&mut base.original_release_date),
        OverlayField::Description => Some(&mut base.description),
        OverlayField::Comment => Some(&mut base.comment),
        OverlayField::SortTitle => Some(&mut base.sort_title),
        _ => None,
    }
}

fn record(provenance: &mut Option<MetadataProvenance>, field: OverlayField, source: FieldSource) {
    provenance
        .get_or_insert_default()
        .insert(field.key().into(), vec![source]);
}

impl OverlayMetadata for AlbumMetadata {
    fn apply_field(&mut self, field: OverlayField, value: Option<String>, source: FieldSource) {
        match field {
            OverlayField::Title => {
                if let Some(value) = value {
                    self.title = value;
                } else {
                    return;
                }
            }
            OverlayField::CatalogNumber => self.catalog_number = value,
            OverlayField::Label => self.labels = value.map(|value| vec![value]),
            OverlayField::ReleaseType => {
                self.release_type = value.as_deref().and_then(ReleaseType::from_label)
            }
            OverlayField::Barcode => self.barcode = value,
            OverlayField::ReleaseCountry => self.release_country = value,
            field => {
                if let Some(target) = common(&mut self.base, field) {
                    *target = value;
                } else {
                    return;
                }
            }
        }
        record(&mut self.base.provenance, field, source);
    }
}

impl OverlayMetadata for DiscMetadata {
    fn apply_field(&mut self, field: OverlayField, value: Option<String>, source: FieldSource) {
        if field == OverlayField::Title {
            self.title = value;
        } else if let Some(target) = common(&mut self.base, field) {
            *target = value;
        } else {
            return;
        }
        record(&mut self.base.provenance, field, source);
    }
}

impl OverlayMetadata for AudioMetadata {
    fn apply_field(&mut self, field: OverlayField, value: Option<String>, source: FieldSource) {
        match field {
            OverlayField::Title => {
                if let Some(value) = value {
                    self.title = value;
                } else {
                    return;
                }
            }
            OverlayField::Subtitle => self.subtitle = value,
            field => {
                if let Some(target) = common(&mut self.base, field) {
                    *target = value;
                } else {
                    return;
                }
            }
        }
        record(&mut self.base.provenance, field, source);
    }
}

impl OverlayMetadata for ReleaseTrackMetadata {
    fn apply_field(&mut self, field: OverlayField, value: Option<String>, source: FieldSource) {
        if field == OverlayField::Title
            && let Some(title) = value
        {
            self.title = title;
            record(&mut self.provenance, field, source);
        }
    }
}
