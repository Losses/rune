use std::borrow::Cow;

use bytemuck::{Pod, Zeroable};
use rand::Rng;

use super::{two_means_binary_quantized as two_means, Manhattan};
use crate::distance::Distance;
use crate::node::Leaf;
use crate::parallel::ImmutableSubsetLeafs;
use crate::spaces::simple::dot_product_binary_quantized;
use crate::unaligned_vector::{self, BinaryQuantized, UnalignedVector};

/// A taxicab geometry or a Manhattan geometry is a geometry whose usual distance function
/// or metric of Euclidean geometry is replaced by a new metric in which the distance between
/// two points is the sum of the absolute differences of their Cartesian coordinates.
/// /!\ This distance function is binary quantized, which means it loses all its precision
///     and their scalar values are converted to `-1` or `1`.
#[derive(Debug, Clone)]
pub enum BinaryQuantizedManhattan {}

/// The header of BinaryQuantizedEuclidean leaf nodes.
#[repr(C)]
#[derive(Pod, Zeroable, Debug, Clone, Copy)]
pub struct NodeHeaderBinaryQuantizedManhattan {
    /// An extra constant term to determine the offset of the plane
    bias: f32,
}

impl Distance for BinaryQuantizedManhattan {
    const DEFAULT_OVERSAMPLING: usize = 3;

    type Header = NodeHeaderBinaryQuantizedManhattan;
    type VectorCodec = unaligned_vector::BinaryQuantized;

    fn name() -> &'static str {
        "binary quantized manhattan"
    }

    fn new_header(_vector: &UnalignedVector<Self::VectorCodec>) -> Self::Header {
        NodeHeaderBinaryQuantizedManhattan { bias: 0.0 }
    }

    fn built_distance(p: &Leaf<Self>, q: &Leaf<Self>) -> f32 {
        manhattan_distance_binary_quantized(&p.vector, &q.vector)
    }

    /// Normalizes the distance returned by the distance method.
    fn normalized_distance(d: f32, dimensions: usize) -> f32 {
        d.max(0.0) / dimensions as f32
    }

    fn norm_no_header(v: &UnalignedVector<Self::VectorCodec>) -> f32 {
        let ones = v
            .as_bytes()
            .iter()
            .map(|b| b.count_ones() as i32 - b.count_zeros() as i32)
            .sum::<i32>() as f32;
        ones.sqrt()
    }

    fn init(_node: &mut Leaf<Self>) {}

    fn create_split<'a, R: Rng>(
        children: &'a ImmutableSubsetLeafs<Self>,
        rng: &mut R,
    ) -> heed::Result<Cow<'a, UnalignedVector<Self::VectorCodec>>> {
        let [node_p, node_q] = two_means::<Self, Manhattan, R>(rng, children, false)?;
        let vector: Vec<f32> =
            node_p.vector.iter().zip(node_q.vector.iter()).map(|(p, q)| p - q).collect();
        let mut normal = Leaf {
            header: NodeHeaderBinaryQuantizedManhattan { bias: 0.0 },
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

/// For the binary quantized manhattan distance:
/// ```text
/// p.vector.iter().zip(q.vector.iter()).map(|(p, q)| (p - q).abs()).sum()
/// ```
/// 1. We need to subtract two scalars and take the absolute value:
/// ```text
/// -1 - -1 =  0 | abs => 0
/// -1 -  1 = -2 | abs => 2
///  1 - -1 =  2 | abs => 2
///  1 -  1 =  0 | abs => 0
/// ```
///
/// It's very similar to the euclidean distance.
/// => It's a xor, we counts the `1`s and multiplicate the result by `2` at the end.
fn manhattan_distance_binary_quantized(
    u: &UnalignedVector<BinaryQuantized>,
    v: &UnalignedVector<BinaryQuantized>,
) -> f32 {
    let ret =
        u.as_bytes().iter().zip(v.as_bytes()).map(|(u, v)| (u ^ v).count_ones()).sum::<u32>() * 2;
    ret as f32
}
