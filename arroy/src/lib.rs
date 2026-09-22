//! Arroy ([Approximate Rearest Reighbors][1] Oh Yeah) is a Rust library with the interface of the [Annoy Python library][2]
//! to search for vectors in space that are close to a given query vector. It is based on LMDB, a memory-mapped key-value store,
//! so many processes may share the same data and atomically modify the vectors.
//!
//! [1]: https://en.wikipedia.org/wiki/Nearest_neighbor_search#Approximate_nearest_neighbor
//! [2]: https://github.com/spotify/annoy/#full-python-api
//!
//! # Examples
//!
//! Open an LMDB database, store some vectors in it and query the top 20 nearest items from the first vector. This is the most
//! trivial way to use arroy and it's fairly easy. Just do not forget to [`ArroyBuilder::build`] and [`heed::RwTxn::commit`]
//! when you are done inserting your items.
//!
//! ```
//! use std::num::NonZeroUsize;
//!
//! use arroy::distances::Euclidean;
//! use arroy::{Database as ArroyDatabase, Writer, Reader};
//! use rand::rngs::StdRng;
//! use rand::{Rng, SeedableRng};
//!
//! /// That's the 200MiB size limit we allow LMDB to grow.
//! const TWENTY_HUNDRED_MIB: usize = 2 * 1024 * 1024 * 1024;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let dir = tempfile::tempdir()?;
//! let env = unsafe { heed::EnvOpenOptions::new().map_size(TWENTY_HUNDRED_MIB).open(dir.path()) }?;
//!
//! // we will open the default LMDB unnamed database
//! let mut wtxn = env.write_txn()?;
//! let db: ArroyDatabase<Euclidean> = env.create_database(&mut wtxn, None)?;
//!
//! // Now we can give it to our arroy writer
//! let index = 0;
//! let dimensions = 5;
//! let writer = Writer::<Euclidean>::new(db, index, dimensions);
//!
//! // let's write some vectors
//! writer.add_item(&mut wtxn, 0,    &[0.8,  0.49, 0.27, 0.76, 0.94])?;
//! writer.add_item(&mut wtxn, 1,    &[0.66, 0.86, 0.42, 0.4,  0.31])?;
//! writer.add_item(&mut wtxn, 2,    &[0.5,  0.95, 0.7,  0.51, 0.03])?;
//! writer.add_item(&mut wtxn, 100,  &[0.52, 0.33, 0.65, 0.23, 0.44])?;
//! writer.add_item(&mut wtxn, 1000, &[0.18, 0.43, 0.48, 0.81, 0.29])?;
//!
//! // You can specify the number of trees to use or specify None.
//! let mut rng = StdRng::seed_from_u64(42);
//! writer.builder(&mut rng).build(&mut wtxn)?;
//!
//! // By committing, other readers can query the database in parallel.
//! wtxn.commit()?;
//!
//! let mut rtxn = env.read_txn()?;
//! let reader = Reader::<Euclidean>::open(&rtxn, index, db)?;
//! let n_results = 20;
//!
//! let mut query = reader.nns(n_results);
//!
//! // You can increase the quality of the results by forcing arroy to search into more nodes.
//! // This multiplier is arbitrary but basically the higher, the better the results, the slower the query.
//! let is_precise = true;
//! if is_precise {
//!     query.search_k(NonZeroUsize::new(n_results * reader.n_trees() * 15).unwrap());
//! }
//!
//! // Similar searching can be achieved by requesting the nearest neighbors of a given item.
//! let item_id = 0;
//! let arroy_results = query.by_item(&rtxn, item_id)?.unwrap();
//! # Ok(()) }
//! ```

#![warn(missing_docs)]
#![doc(
    html_favicon_url = "https://raw.githubusercontent.com/meilisearch/arroy/main/assets/arroy-electric-clusters.ico?raw=true"
)]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/meilisearch/arroy/main/assets/arroy-electric-clusters-logo.png?raw=true"
)]

mod distance;
mod error;
mod item_iter;
mod key;
mod metadata;
mod node;
mod node_id;
mod parallel;
mod reader;
mod roaring;
mod spaces;
mod stats;
pub mod upgrade;
mod version;
mod writer;

#[cfg(test)]
mod tests;
mod unaligned_vector;

pub use distance::Distance;
pub use error::Error;

use key::{Key, Prefix, PrefixCodec};
use metadata::{Metadata, MetadataCodec};
use node::{Node, NodeCodec};
use node_id::{NodeId, NodeMode};
pub use reader::{QueryBuilder, Reader};
pub use stats::{Stats, TreeStats};
pub use writer::{ArroyBuilder, MainStep, SubStep, Writer, WriterProgress};

/// The set of types used by the [`Distance`] trait.
pub mod internals {
    use rand::Rng;

    pub use crate::distance::{
        NodeHeaderBinaryQuantizedCosine, NodeHeaderBinaryQuantizedEuclidean,
        NodeHeaderBinaryQuantizedManhattan, NodeHeaderCosine, NodeHeaderDotProduct,
        NodeHeaderEuclidean, NodeHeaderManhattan,
    };
    pub use crate::key::KeyCodec;
    pub use crate::node::{Leaf, NodeCodec};
    pub use crate::unaligned_vector::{SizeMismatch, UnalignedVector, UnalignedVectorCodec};

    /// A type that is used to decide on
    /// which side of a plane we move an item.
    #[derive(Debug, Copy, Clone)]
    pub enum Side {
        /// The left side.
        Left,
        /// The right side.
        Right,
    }

    impl Side {
        pub(crate) fn random<R: Rng>(rng: &mut R) -> Side {
            if rng.gen() {
                Side::Left
            } else {
                Side::Right
            }
        }
    }
}

/// The set of distances implementing the [`Distance`] and supported by arroy.
pub mod distances {
    pub use crate::distance::{
        BinaryQuantizedCosine, BinaryQuantizedEuclidean, BinaryQuantizedManhattan, Cosine,
        DotProduct, Euclidean, Manhattan,
    };
}

/// A custom Result type that is returning an arroy error by default.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The database required by arroy for reading or writing operations.
pub type Database<D> = heed::Database<internals::KeyCodec, NodeCodec<D>>;

/// An identifier for the items stored in the database.
pub type ItemId = u32;
