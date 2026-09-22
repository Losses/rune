use std::borrow::Cow;

use bytemuck::{Pod, Zeroable};
use rand::Rng;

use super::{two_means_binary_quantized as two_means, Euclidean};
use crate::distance::Distance;
use crate::node::Leaf;
use crate::parallel::ImmutableSubsetLeafs;
use crate::spaces::simple::dot_product_binary_quantized;
use crate::unaligned_vector::{self, BinaryQuantized, UnalignedVector};

/// The Euclidean distance between two points in Euclidean space
/// is the length of the line segment between them.
///
/// `d(p, q) = sqrt((p - q)²)`
/// /!\ This distance function is binary quantized, which means it loses all its precision
///     and their scalar values are converted to `-1` or `1`.
#[derive(Debug, Clone)]
pub enum BinaryQuantizedEuclidean {}

/// The header of `BinaryQuantizedEuclidean` leaf nodes.
#[repr(C)]
#[derive(Pod, Zeroable, Debug, Clone, Copy)]
pub struct NodeHeaderBinaryQuantizedEuclidean {
    /// An extra constant term to determine the offset of the plane
    bias: f32,
}

impl Distance for BinaryQuantizedEuclidean {
    const DEFAULT_OVERSAMPLING: usize = 3;

    type Header = NodeHeaderBinaryQuantizedEuclidean;
    type VectorCodec = unaligned_vector::BinaryQuantized;

    fn name() -> &'static str {
        "binary quantized euclidean"
    }

    fn new_header(_vector: &UnalignedVector<Self::VectorCodec>) -> Self::Header {
        NodeHeaderBinaryQuantizedEuclidean { bias: 0.0 }
    }

    fn built_distance(p: &Leaf<Self>, q: &Leaf<Self>) -> f32 {
        squared_euclidean_distance_binary_quantized(&p.vector, &q.vector)
    }

    /// Normalizes the distance returned by the distance method.
    fn normalized_distance(d: f32, dimensions: usize) -> f32 {
        d / dimensions as f32
    }

    fn norm_no_header(v: &UnalignedVector<Self::VectorCodec>) -> f32 {
        dot_product_binary_quantized(v, v).sqrt()
    }

    fn init(_node: &mut Leaf<Self>) {}

    fn create_split<'a, R: Rng>(
        children: &'a ImmutableSubsetLeafs<Self>,
        rng: &mut R,
    ) -> heed::Result<Cow<'a, UnalignedVector<Self::VectorCodec>>> {
        let [node_p, node_q] = two_means::<Self, Euclidean, R>(rng, children, false)?;
        let vector: Vec<f32> =
            node_p.vector.iter().zip(node_q.vector.iter()).map(|(p, q)| p - q).collect();
        let mut normal = Leaf {
            header: NodeHeaderBinaryQuantizedEuclidean { bias: 0.0 },
            vector: UnalignedVector::from_slice(&vector),
        };
        Self::normalize(&mut normal);

        Ok(Cow::Owned(normal.vector.into_owned()))
    }

    fn margin(p: &Leaf<Self>, q: &Leaf<Self>) -> f32 {
        p.header.bias + dot_product_binary_quantized(&p.vector, &q.vector)
    }

    fn margin_no_header(
        p: &UnalignedVector<Self::VectorCodec>,
        q: &UnalignedVector<Self::VectorCodec>,
    ) -> f32 {
        dot_product_binary_quantized(p, q)
    }
}

/// For the binary quantized squared euclidean distance:
/// 1. We need to do the following operation: `(u - v)^2`, in our case the only allowed values are `-1` and `1`:
/// ```text
/// -1 - -1 =  0 | ^2 => 0
/// -1 -  1 = -2 | ^2 => 4
///  1 - -1 =  2 | ^2 => 4
///  1 -  1 =  0 | ^2 => 0
/// ```
///
/// If we replace the `-1` by the binary quantized `0`, and the `1` stays `1`s:
/// ```text
/// 0 ^ 0 = 0
/// 0 ^ 1 = 1
/// 1 ^ 0 = 1
/// 1 ^ 1 = 0
/// ```
///
/// The result must be multiplicated by `4`. But that can be done at the very end.
///
/// 2. Then we need to do the sum of the results:
///    Since we cannot go into the negative, it's safe to hold everything in a `u32` and simply counts the `1`s.
///    At the very end, before converting the value to a `f32` we can multiply everything by 4.
fn squared_euclidean_distance_binary_quantized(
    u: &UnalignedVector<BinaryQuantized>,
    v: &UnalignedVector<BinaryQuantized>,
) -> f32 {
    let ret =
        u.as_bytes().iter().zip(v.as_bytes()).map(|(u, v)| (u ^ v).count_ones()).sum::<u32>() * 4;
    ret as f32
}
