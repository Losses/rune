use std::borrow::Cow;
use std::ffi::CStr;
use std::mem::size_of;

use ::roaring::RoaringBitmap;
use byteorder::{BigEndian, ByteOrder};
use heed::BoxedError;

use crate::node::ItemIds;

#[derive(Debug)]
pub struct Metadata<'a> {
    pub dimensions: u32,
    pub items: RoaringBitmap,
    pub roots: ItemIds<'a>,
    pub distance: &'a str,
}

pub enum MetadataCodec {}

impl<'a> heed::BytesEncode<'a> for MetadataCodec {
    type EItem = Metadata<'a>;

    fn bytes_encode(item: &'a Self::EItem) -> Result<Cow<'a, [u8]>, BoxedError> {
        let Metadata { dimensions, items, roots, distance } = item;
        debug_assert!(!distance.as_bytes().iter().any(|&b| b == 0));

        let mut output = Vec::with_capacity(
            size_of::<u32>()
                + items.serialized_size()
                + roots.len() * size_of::<u32>()
                + distance.len()
                + 1,
        );
        output.extend_from_slice(distance.as_bytes());
        output.push(0);
        output.extend_from_slice(&dimensions.to_be_bytes());
        output.extend_from_slice(&(items.serialized_size() as u32).to_be_bytes());
        items.serialize_into(&mut output)?;
        output.extend_from_slice(roots.raw_bytes());

        Ok(Cow::Owned(output))
    }
}

impl<'a> heed::BytesDecode<'a> for MetadataCodec {
    type DItem = Metadata<'a>;

    fn bytes_decode(bytes: &'a [u8]) -> Result<Self::DItem, BoxedError> {
        let distance = CStr::from_bytes_until_nul(bytes)?.to_str()?;
        let bytes = &bytes[distance.len() + 1..];
        let dimensions = BigEndian::read_u32(bytes);
        let bytes = &bytes[size_of::<u32>()..];
        let items_size = BigEndian::read_u32(bytes) as usize;
        let bytes = &bytes[size_of::<u32>()..];
        let items = RoaringBitmap::deserialize_from(&bytes[..items_size])?;
        let bytes = &bytes[items_size..];

        Ok(Metadata { dimensions, items, roots: ItemIds::from_bytes(bytes), distance })
    }
}

#[cfg(test)]
mod test {
    use heed::{BytesDecode, BytesEncode};

    use super::*;

    #[test]
    fn metadata_codec() {
        let metadata = Metadata {
            dimensions: 12,
            items: RoaringBitmap::from_sorted_iter(0..100).unwrap(),
            roots: ItemIds::from_slice(&[1, 2, 3, 4]),
            distance: "tamo",
        };

        let encoded = MetadataCodec::bytes_encode(&metadata).unwrap();
        let decoded = MetadataCodec::bytes_decode(&encoded).unwrap();

        assert_eq!(metadata.dimensions, decoded.dimensions);
        assert_eq!(metadata.items, decoded.items);
        assert_eq!(metadata.roots.raw_bytes(), decoded.roots.raw_bytes());
        assert_eq!(metadata.distance, decoded.distance);
    }
}
