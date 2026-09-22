use std::borrow::Cow;
use std::fmt;

pub use binary_quantized_cosine::{BinaryQuantizedCosine, NodeHeaderBinaryQuantizedCosine};
pub use binary_quantized_euclidean::{
    BinaryQuantizedEuclidean, NodeHeaderBinaryQuantizedEuclidean,
};
pub use binary_quantized_manhattan::{
    BinaryQuantizedManhattan, NodeHeaderBinaryQuantizedManhattan,
};
use bytemuck::{Pod, Zeroable};
pub use cosine::{Cosine, NodeHeaderCosine};
pub use dot_product::{DotProduct, NodeHeaderDotProduct};
pub use euclidean::{Euclidean, NodeHeaderEuclidean};
use heed::{RwPrefix, RwTxn};
pub use manhattan::{Manhattan, NodeHeaderManhattan};
use rand::Rng;

use crate::internals::{KeyCodec, Side};
use crate::node::Leaf;
use crate::parallel::ImmutableSubsetLeafs;
use crate::unaligned_vector::{UnalignedVector, UnalignedVectorCodec};
use crate::NodeCodec;

mod binary_quantized_cosine;
mod binary_quantized_euclidean;
mod binary_quantized_manhattan;
mod cosine;
mod dot_product;
mod euclidean;
mod manhattan;

fn new_leaf<D: Distance>(vec: Vec<f32>) -> Leaf<'static, D> {
    let vector = UnalignedVector::from_vec(vec);
    Leaf { header: D::new_header(&vector), vector }
}

/// A trait used by arroy to compute the distances,
/// compute the split planes, and normalize user vectors.
#[allow(missing_docs)]
pub trait Distance: Send + Sync + Sized + Clone + fmt::Debug + 'static {
    const DEFAULT_OVERSAMPLING: usize = 1;

    /// A header structure with informations related to the
    type Header: Pod + Zeroable + fmt::Debug;
    type VectorCodec: UnalignedVectorCodec;

    fn name() -> &'static str;

    fn new_header(vector: &UnalignedVector<Self::VectorCodec>) -> Self::Header;

    /// Returns a non-normalized distance.
    fn built_distance(p: &Leaf<Self>, q: &Leaf<Self>) -> f32;

    fn non_built_distance(p: &Leaf<Self>, q: &Leaf<Self>) -> f32 {
        Self::built_distance(p, q)
    }

    /// Normalizes the distance returned by the distance method.
    fn normalized_distance(d: f32, _dimensions: usize) -> f32 {
        d.sqrt()
    }

    fn pq_distance(distance: f32, margin: f32, side: Side) -> f32 {
        match side {
            Side::Left => (-margin).min(distance),
            Side::Right => margin.min(distance),
        }
    }

    fn norm(leaf: &Leaf<Self>) -> f32 {
        Self::norm_no_header(&leaf.vector)
    }

    fn norm_no_header(v: &UnalignedVector<Self::VectorCodec>) -> f32;

    fn normalize(node: &mut Leaf<Self>) {
        let norm = Self::norm(node);
        if norm > 0.0 {
            let vec: Vec<_> = node.vector.iter().map(|x| x / norm).collect();
            node.vector = UnalignedVector::from_vec(vec);
        }
    }

    fn init(node: &mut Leaf<Self>);

    fn update_mean(mean: &mut Leaf<Self>, new_node: &Leaf<Self>, norm: f32, c: f32) {
        let vec: Vec<_> = mean
            .vector
            .iter()
            .zip(new_node.vector.iter())
            .map(|(x, n)| (x * c + n / norm) / (c + 1.0))
            .collect();
        mean.vector = UnalignedVector::from_vec(vec);
    }

    fn create_split<'a, R: Rng>(
        children: &'a ImmutableSubsetLeafs<Self>,
        rng: &mut R,
    ) -> heed::Result<Cow<'a, UnalignedVector<Self::VectorCodec>>>;

    fn margin(p: &Leaf<Self>, q: &Leaf<Self>) -> f32 {
        Self::margin_no_header(&p.vector, &q.vector)
    }

    fn margin_no_header(
        p: &UnalignedVector<Self::VectorCodec>,
        q: &UnalignedVector<Self::VectorCodec>,
    ) -> f32;

    fn side<R: Rng>(
        normal_plane: &UnalignedVector<Self::VectorCodec>,
        node: &Leaf<Self>,
        rng: &mut R,
    ) -> Side {
        let dot = Self::margin_no_header(&node.vector, normal_plane);
        if dot > 0.0 {
            Side::Right
        } else if dot < 0.0 {
            Side::Left
        } else {
            Side::random(rng)
        }
    }

    fn preprocess(
        _wtxn: &mut RwTxn,
        _new_iter: impl for<'a> Fn(
            &'a mut RwTxn,
        ) -> heed::Result<RwPrefix<'a, KeyCodec, NodeCodec<Self>>>,
    ) -> heed::Result<()> {
        Ok(())
    }
}

fn two_means<D: Distance, R: Rng>(
    rng: &mut R,
    leafs: &ImmutableSubsetLeafs<D>,
    cosine: bool,
) -> heed::Result<[Leaf<'static, D>; 2]> {
    // This algorithm is a huge heuristic. Empirically it works really well, but I
    // can't motivate it well. The basic idea is to keep two centroids and assign
    // points to either one of them. We weight each centroid by the number of points
    // assigned to it, so to balance it.

    const ITERATION_STEPS: usize = 200;

    let [leaf_p, leaf_q] = leafs.choose_two(rng)?.unwrap();
    let (mut leaf_p, mut leaf_q) = (leaf_p.into_owned(), leaf_q.into_owned());

    if cosine {
        D::normalize(&mut leaf_p);
        D::normalize(&mut leaf_q);
    }

    D::init(&mut leaf_p);
    D::init(&mut leaf_q);

    let mut ic = 1.0;
    let mut jc = 1.0;
    for _ in 0..ITERATION_STEPS {
        let node_k = leafs.choose(rng)?.unwrap();
        let di = ic * D::non_built_distance(&leaf_p, &node_k);
        let dj = jc * D::non_built_distance(&leaf_q, &node_k);
        let norm = if cosine { D::norm(&node_k) } else { 1.0 };
        if norm.is_nan() || norm <= 0.0 {
            continue;
        }
        if di < dj {
            Distance::update_mean(&mut leaf_p, &node_k, norm, ic);
            Distance::init(&mut leaf_p);
            ic += 1.0;
        } else if dj < di {
            Distance::update_mean(&mut leaf_q, &node_k, norm, jc);
            Distance::init(&mut leaf_q);
            jc += 1.0;
        }
    }

    Ok([leaf_p, leaf_q])
}

pub fn two_means_binary_quantized<D: Distance, NonBqDist: Distance, R: Rng>(
    rng: &mut R,
    leafs: &ImmutableSubsetLeafs<D>,
    cosine: bool,
) -> heed::Result<[Leaf<'static, NonBqDist>; 2]> {
    // This algorithm is a huge heuristic. Empirically it works really well, but I
    // can't motivate it well. The basic idea is to keep two centroids and assign
    // points to either one of them. We weight each centroid by the number of points
    // assigned to it, so to balance it.
    // Even though the points we're working on are binary quantized, for the centroid
    // to move, we need to store it as f32. This requires us to convert every binary quantized
    // vectors to f32 vectors, but the recall suffers too much if we don't do it.

    const ITERATION_STEPS: usize = 200;

    let [leaf_p, leaf_q] = leafs.choose_two(rng)?.unwrap();
    let mut leaf_p: Leaf<'static, NonBqDist> = new_leaf(leaf_p.vector.to_vec());
    let mut leaf_q: Leaf<'static, NonBqDist> = new_leaf(leaf_q.vector.to_vec());

    if cosine {
        NonBqDist::normalize(&mut leaf_p);
        NonBqDist::normalize(&mut leaf_q);
    }

    NonBqDist::init(&mut leaf_p);
    NonBqDist::init(&mut leaf_q);

    let mut ic = 1.0;
    let mut jc = 1.0;
    for _ in 0..ITERATION_STEPS {
        let node_k = leafs.choose(rng)?.unwrap();
        let node_k: Leaf<'static, NonBqDist> = new_leaf(node_k.vector.to_vec());
        let di = ic * NonBqDist::non_built_distance(&leaf_p, &node_k);
        let dj = jc * NonBqDist::non_built_distance(&leaf_q, &node_k);
        let norm = if cosine { NonBqDist::norm(&node_k) } else { 1.0 };
        if norm.is_nan() || norm <= 0.0 {
            continue;
        }
        if di < dj {
            Distance::update_mean(&mut leaf_p, &node_k, norm, ic);
            Distance::init(&mut leaf_p);
            ic += 1.0;
        } else if dj < di {
            Distance::update_mean(&mut leaf_q, &node_k, norm, jc);
            Distance::init(&mut leaf_q);
            jc += 1.0;
        }
    }

    Ok([leaf_p, leaf_q])
}
