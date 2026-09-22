use std::any::TypeId;
use std::borrow::Cow;
use std::mem;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use heed::types::{Bytes, DecodeIgnore, Unit};
use heed::{MdbError, PutFlags, RoTxn, RwTxn};
use rand::{Rng, SeedableRng};
use rayon::iter::repeatn;
use rayon::prelude::*;
use roaring::RoaringBitmap;

use crate::distance::Distance;
use crate::internals::{KeyCodec, Side};
use crate::item_iter::ItemIter;
use crate::node::{Descendants, ItemIds, Leaf, SplitPlaneNormal};
use crate::node_id::NodeMode;
use crate::parallel::{
    ConcurrentNodeIds, ImmutableLeafs, ImmutableSubsetLeafs, ImmutableTrees, TmpNodes,
    TmpNodesReader,
};
use crate::reader::item_leaf;
use crate::unaligned_vector::UnalignedVector;
use crate::version::{Version, VersionCodec};
use crate::{
    Database, Error, ItemId, Key, Metadata, MetadataCodec, Node, NodeCodec, NodeId, Prefix,
    PrefixCodec, Result,
};

/// The options available when building the arroy database.
pub struct ArroyBuilder<'a, D: Distance, R: Rng + SeedableRng> {
    writer: &'a Writer<D>,
    rng: &'a mut R,
    inner: BuildOption<'a>,
}

/// Helps you understand what is happening inside of arroy during an indexing process.
#[derive(Debug)]
pub struct WriterProgress {
    /// The `main` part describes what's going on overall.
    pub main: MainStep,
    /// Sometimes, when a part takes a lot of time, you'll get a substep describing with more details what's going on.
    pub sub: Option<SubStep>,
}

/// When a `MainStep` takes too long, it may output a sub-step that gives you more details about the progression we've made on the current step.
#[derive(Debug)]
pub struct SubStep {
    /// The name of what is being updated.
    pub unit: &'static str,
    /// The `current` iteration we're at. It's stored in an `AtomicU32` so arroy can update it very quickly without calling your closure again.
    pub current: Arc<AtomicU32>,
    /// The `max`imum number of iteration it'll do before updating the `MainStep` again.
    pub max: u32,
}

impl SubStep {
    fn new(unit: &'static str, max: u32) -> (Self, Arc<AtomicU32>) {
        let current = Arc::new(AtomicU32::new(0));
        (Self { unit, current: current.clone(), max }, current)
    }
}

/// Some steps arroy will go through during an indexing process.
/// Some steps may be skipped in certain cases, and the name of the variant
/// the order in which they appear and the time they take is unspecified, and
/// might change from one version to the next one.
/// It's recommended not to assume anything from this enum.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, enum_iterator::Sequence)]
#[allow(missing_docs)]
pub enum MainStep {
    PreProcessingTheItems,
    WritingTheDescendantsAndMetadata,
    RetrieveTheUpdatedItems,
    RetrievingTheTreeAndItemNodes,
    UpdatingTheTrees,
    CreateNewTrees,
    WritingNodesToDatabase,
    DeleteExtraneousTrees,
    WriteTheMetadata,
}

/// The options available when building the arroy database.
struct BuildOption<'a> {
    n_trees: Option<usize>,
    split_after: Option<usize>,
    available_memory: Option<usize>,
    cancel: Box<dyn Fn() -> bool + 'a + Sync + Send>,
    progress: Box<dyn Fn(WriterProgress) + 'a + Sync + Send>,
}

impl Default for BuildOption<'_> {
    fn default() -> Self {
        Self {
            n_trees: None,
            split_after: None,
            available_memory: None,
            cancel: Box::new(|| false),
            progress: Box::new(|_| ()),
        }
    }
}

impl<'a, D: Distance, R: Rng + SeedableRng> ArroyBuilder<'a, D, R> {
    /// The number of trees to build. If not set arroy will determine the best amount to build for your number of vectors by itself.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use arroy::{Writer, distances::Euclidean};
    /// # let (writer, wtxn): (Writer<Euclidean>, heed::RwTxn) = todo!();
    /// use rand::rngs::StdRng;
    /// use rand::SeedableRng;
    /// let mut rng = StdRng::seed_from_u64(13);
    /// writer.builder(&mut rng).n_trees(10).build(&mut wtxn);
    /// ```
    pub fn n_trees(&mut self, n_trees: usize) -> &mut Self {
        self.inner.n_trees = Some(n_trees);
        self
    }

    /// Configure the maximum number of items stored in a descendant node.
    ///
    /// This is only applied to the newly created or updated tree node.
    /// If the value is modified while working on an already existing database,
    /// the nodes that don't need to be updated won't be recreated.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use arroy::{Writer, distances::Euclidean};
    /// # let (writer, wtxn): (Writer<Euclidean>, heed::RwTxn) = todo!();
    /// use rand::rngs::StdRng;
    /// use rand::SeedableRng;
    /// let mut rng = StdRng::seed_from_u64(92);
    /// writer.builder(&mut rng).split_after(1000).build(&mut wtxn);
    /// ```
    pub fn split_after(&mut self, split_after: usize) -> &mut Self {
        self.inner.split_after = Some(split_after);
        self
    }

    /// Configure the maximum memory arroy can use to build its trees in bytes.
    ///
    /// This value is used as a hint; arroy may still consume too much memory, especially if the value is too low.
    /// If not specified, arroy will use as much memory as possible but keep in mind that if arroy tries to use more
    /// memory than you have, it'll become very slow.
    ///
    /// In this case, it will randomly read the disk as pages will be invalidated by other reads, and OS cache will
    /// become useless.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use arroy::{Writer, distances::Euclidean};
    /// # let (writer, wtxn): (Writer<Euclidean>, heed::RwTxn) = todo!();
    /// use rand::rngs::StdRng;
    /// use rand::SeedableRng;
    /// let mut rng = StdRng::seed_from_u64(92);
    /// let memory = 1024 * 1024 * 1024 * 4; // 4 GiB
    /// writer.builder(&mut rng).available_memory(memory).build(&mut wtxn);
    /// ```
    pub fn available_memory(&mut self, memory: usize) -> &mut Self {
        self.inner.available_memory = Some(memory);
        self
    }

    /// Provide a closure that can cancel the indexing process early if needed.
    /// There is no guarantee on when the process is going to cancel itself, but
    /// arroy will try to stop as soon as possible once the closure returns `true`.
    ///
    /// Since the closure is not mutable and will be called from multiple threads
    /// at the same time it's encouraged to make it quick to execute. A common
    /// way to use it is to fetch an `AtomicBool` inside it that can be set
    /// from another thread without lock.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use arroy::{Writer, distances::Euclidean};
    /// # let (writer, wtxn): (Writer<Euclidean>, heed::RwTxn) = todo!();
    /// use rand::rngs::StdRng;
    /// use rand::SeedableRng;
    /// use std::sync::atomic::{AtomicBool, Ordering};
    ///
    /// let stops_after = AtomicBool::new(false);
    ///
    /// // Cancel the task after one minute
    /// std::thread::spawn(|| {
    ///     let one_minute = std::time::Duration::from_secs(60);
    ///     std::thread::sleep(one_minute);
    ///     stops_after.store(true, Ordering::Relaxed);
    /// });
    ///
    /// let mut rng = StdRng::seed_from_u64(92);
    /// writer.builder(&mut rng).cancel(|| stops_after.load(Ordering::Relaxed)).build(&mut wtxn);
    /// ```
    pub fn cancel(&mut self, cancel: impl Fn() -> bool + 'a + Sync + Send) -> &mut Self {
        self.inner.cancel = Box::new(cancel);
        self
    }

    /// The provided closure is called between all the indexing steps.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use arroy::{Writer, distances::Euclidean};
    /// # let (writer, wtxn): (Writer<Euclidean>, heed::RwTxn) = todo!();
    /// use rand::rngs::StdRng;
    /// use rand::SeedableRng;
    /// use std::sync::atomic::{AtomicBool, Ordering};
    ///
    /// let mut rng = StdRng::seed_from_u64(4729);
    /// writer.builder(&mut rng).progress(|progress| println!("{progress:?}")).build(&mut wtxn);
    /// ```
    pub fn progress(&mut self, progress: impl Fn(WriterProgress) + 'a + Sync + Send) -> &mut Self {
        self.inner.progress = Box::new(progress);
        self
    }

    /// Generates a forest of `n_trees` trees.
    ///
    /// More trees give higher precision when querying at the cost of more disk usage.
    ///
    /// This function is using rayon to spawn threads. It can be configured
    /// by using the [`rayon::ThreadPoolBuilder`] and the
    /// [`rayon::ThreadPool::install`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use arroy::{Writer, distances::Euclidean};
    /// # let (writer, wtxn): (Writer<Euclidean>, heed::RwTxn) = todo!();
    /// use rand::rngs::StdRng;
    /// use rand::SeedableRng;
    /// let mut rng = StdRng::seed_from_u64(92);
    /// writer.builder(&mut rng).build(&mut wtxn);
    /// ```
    pub fn build(&mut self, wtxn: &mut RwTxn) -> Result<()> {
        self.writer.build(wtxn, self.rng, &self.inner)
    }
}

/// A writer to store new items, remove existing ones,
/// and build the search tree to query the nearest
/// neighbors to items or vectors.
#[derive(Debug)]
pub struct Writer<D: Distance> {
    database: Database<D>,
    index: u16,
    dimensions: usize,
    /// The folder in which tempfile will write its temporary files.
    tmpdir: Option<PathBuf>,
}

impl<D: Distance> Writer<D> {
    /// Creates a new writer from a database, index and dimensions.
    pub fn new(database: Database<D>, index: u16, dimensions: usize) -> Writer<D> {
        let database: Database<D> = database.remap_data_type();
        Writer { database, index, dimensions, tmpdir: None }
    }

    /// Returns a writer after having deleted the tree nodes and rewrote all the items
    /// for the new [`Distance`] format to be able to modify items safely.
    pub fn prepare_changing_distance<ND: Distance>(self, wtxn: &mut RwTxn) -> Result<Writer<ND>> {
        if TypeId::of::<ND>() != TypeId::of::<D>() {
            clear_tree_nodes(wtxn, self.database, self.index)?;

            let mut cursor = self
                .database
                .remap_key_type::<PrefixCodec>()
                .prefix_iter_mut(wtxn, &Prefix::item(self.index))?
                .remap_key_type::<KeyCodec>();
            while let Some((item_id, node)) = cursor.next().transpose()? {
                match node {
                    Node::Leaf(Leaf { header: _, vector }) => {
                        let vector = vector.to_vec();
                        let vector = UnalignedVector::from_vec(vector);
                        let new_leaf = Node::Leaf(Leaf { header: ND::new_header(&vector), vector });
                        unsafe {
                            // safety: We do not keep a reference to the current value, we own it.
                            cursor.put_current_with_options::<NodeCodec<ND>>(
                                PutFlags::empty(),
                                &item_id,
                                &new_leaf,
                            )?
                        };
                    }
                    Node::Descendants(_) | Node::SplitPlaneNormal(_) => panic!(),
                }
            }
        }

        let Writer { database, index, dimensions, tmpdir } = self;
        Ok(Writer { database: database.remap_data_type(), index, dimensions, tmpdir })
    }

    /// Specifies the folder in which arroy will write temporary files when building the tree.
    ///
    /// If specified it uses the [`tempfile::tempfile_in`] function, otherwise it will
    /// use the default [`tempfile::tempfile`] function which uses the OS temporary directory.
    pub fn set_tmpdir(&mut self, path: impl Into<PathBuf>) {
        self.tmpdir = Some(path.into());
    }

    /// Returns an `Option`al vector previous stored in this database.
    pub fn item_vector(&self, rtxn: &RoTxn, item: ItemId) -> Result<Option<Vec<f32>>> {
        Ok(item_leaf(self.database, self.index, rtxn, item)?.map(|leaf| {
            let mut vec = leaf.vector.to_vec();
            vec.truncate(self.dimensions);
            vec
        }))
    }

    /// Returns `true` if the index is empty.
    pub fn is_empty(&self, rtxn: &RoTxn) -> Result<bool> {
        self.iter(rtxn).map(|mut iter| iter.next().is_none())
    }

    /// Returns `true` if the index needs to be built before being able to read in it.
    pub fn need_build(&self, rtxn: &RoTxn) -> Result<bool> {
        Ok(self
            .database
            .remap_types::<PrefixCodec, DecodeIgnore>()
            .prefix_iter(rtxn, &Prefix::updated(self.index))?
            .remap_key_type::<KeyCodec>()
            .next()
            .is_some()
            || self
                .database
                .remap_data_type::<DecodeIgnore>()
                .get(rtxn, &Key::metadata(self.index))?
                .is_none())
    }

    /// Returns `true` if the database contains the given item.
    pub fn contains_item(&self, rtxn: &RoTxn, item: ItemId) -> Result<bool> {
        self.database
            .remap_data_type::<DecodeIgnore>()
            .get(rtxn, &Key::item(self.index, item))
            .map(|opt| opt.is_some())
            .map_err(Into::into)
    }

    /// Returns an iterator over the items vector.
    pub fn iter<'t>(&self, rtxn: &'t RoTxn) -> Result<ItemIter<'t, D>> {
        Ok(ItemIter {
            inner: self
                .database
                .remap_key_type::<PrefixCodec>()
                .prefix_iter(rtxn, &Prefix::item(self.index))?
                .remap_key_type::<KeyCodec>(),
        })
    }

    /// Add an item associated to a vector in the database.
    pub fn add_item(&self, wtxn: &mut RwTxn, item: ItemId, vector: &[f32]) -> Result<()> {
        if vector.len() != self.dimensions {
            return Err(Error::InvalidVecDimension {
                expected: self.dimensions,
                received: vector.len(),
            });
        }

        let vector = UnalignedVector::from_slice(vector);
        let leaf = Leaf { header: D::new_header(&vector), vector };
        self.database.put(wtxn, &Key::item(self.index, item), &Node::Leaf(leaf))?;
        self.database.remap_data_type::<Unit>().put(wtxn, &Key::updated(self.index, item), &())?;

        Ok(())
    }

    /// Attempt to append an item into the database. It is generaly faster to append an item than insert it.
    ///
    /// There are two conditions for an item to be successfully appended:
    ///  - The last item ID in the database is smaller than the one appended.
    ///  - The index of the database is the highest one.
    pub fn append_item(&self, wtxn: &mut RwTxn, item: ItemId, vector: &[f32]) -> Result<()> {
        if vector.len() != self.dimensions {
            return Err(Error::InvalidVecDimension {
                expected: self.dimensions,
                received: vector.len(),
            });
        }

        let vector = UnalignedVector::from_slice(vector);
        let leaf = Leaf { header: D::new_header(&vector), vector };
        let key = Key::item(self.index, item);
        match self.database.put_with_flags(wtxn, PutFlags::APPEND, &key, &Node::Leaf(leaf)) {
            Ok(()) => (),
            Err(heed::Error::Mdb(MdbError::KeyExist)) => return Err(Error::InvalidItemAppend),
            Err(e) => return Err(e.into()),
        }
        // We cannot append here because the items appear after the updated keys
        self.database.remap_data_type::<Unit>().put(wtxn, &Key::updated(self.index, item), &())?;

        Ok(())
    }

    /// Deletes an item stored in this database and returns `true` if it existed.
    pub fn del_item(&self, wtxn: &mut RwTxn, item: ItemId) -> Result<bool> {
        if self.database.delete(wtxn, &Key::item(self.index, item))? {
            self.database.remap_data_type::<Unit>().put(
                wtxn,
                &Key::updated(self.index, item),
                &(),
            )?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Removes everything in the database, user items and internal tree nodes.
    pub fn clear(&self, wtxn: &mut RwTxn) -> Result<()> {
        let mut cursor = self
            .database
            .remap_key_type::<PrefixCodec>()
            .prefix_iter_mut(wtxn, &Prefix::all(self.index))?
            .remap_types::<DecodeIgnore, DecodeIgnore>();

        while let Some((_id, _node)) = cursor.next().transpose()? {
            // safety: we don't have any reference to the database
            unsafe { cursor.del_current() }?;
        }

        Ok(())
    }

    fn used_tree_node(&self, rtxn: &RoTxn) -> Result<RoaringBitmap> {
        Ok(self
            .database
            .remap_key_type::<PrefixCodec>()
            .prefix_iter(rtxn, &Prefix::tree(self.index))?
            .remap_types::<KeyCodec, DecodeIgnore>()
            .try_fold(RoaringBitmap::new(), |mut bitmap, used| -> Result<RoaringBitmap> {
                bitmap.insert(used?.0.node.item);
                Ok(bitmap)
            })
            .unwrap_or_default())
    }

    // we simplify the max descendants (_K) thing by considering
    // that we can fit as much descendants as the number of dimensions
    fn fit_in_descendant(&self, opt: &BuildOption, n: u64) -> bool {
        let max_in_descendant = opt.split_after.unwrap_or(self.dimensions) as u64;
        n <= max_in_descendant
    }

    /// Returns an [`ArroyBuilder`] to configure the available options to build the database.
    pub fn builder<'a, R: Rng + SeedableRng>(&'a self, rng: &'a mut R) -> ArroyBuilder<'a, D, R> {
        ArroyBuilder { writer: self, rng, inner: BuildOption::default() }
    }

    fn build<R: Rng + SeedableRng>(
        &self,
        wtxn: &mut RwTxn,
        rng: &mut R,
        options: &BuildOption,
    ) -> Result<()> {
        tracing::debug!("started preprocessing the items...");

        (options.progress)(WriterProgress { main: MainStep::PreProcessingTheItems, sub: None });

        if (options.cancel)() {
            return Err(Error::BuildCancelled);
        }

        D::preprocess(wtxn, |wtxn| {
            Ok(self
                .database
                .remap_key_type::<PrefixCodec>()
                .prefix_iter_mut(wtxn, &Prefix::item(self.index))?
                .remap_key_type::<KeyCodec>())
        })?;

        if (options.cancel)() {
            return Err(Error::BuildCancelled);
        }

        let item_indices = self.item_indices(wtxn)?;
        let n_items = item_indices.len();

        if self.fit_in_descendant(options, item_indices.len()) {
            tracing::debug!("We can fit every elements in a single descendant node, we can skip all the build process");
            (options.progress)(WriterProgress {
                main: MainStep::WritingTheDescendantsAndMetadata,
                sub: None,
            });
            // No item left in the index, we can clear every tree

            self.database.remap_data_type::<Bytes>().delete_range(
                wtxn,
                &(Key::tree(self.index, 0)..=Key::tree(self.index, ItemId::MAX)),
            )?;

            let mut roots = Vec::new();

            if !item_indices.is_empty() {
                // if we have more than 0 elements we need to create a descendant node

                self.database.put(
                    wtxn,
                    &Key::tree(self.index, 0),
                    &Node::Descendants(Descendants { descendants: Cow::Borrowed(&item_indices) }),
                )?;
                roots.push(0);
            }

            tracing::debug!("reset the updated items...");
            let mut updated_iter = self
                .database
                .remap_types::<PrefixCodec, DecodeIgnore>()
                .prefix_iter_mut(wtxn, &Prefix::updated(self.index))?
                .remap_key_type::<KeyCodec>();
            while updated_iter.next().transpose()?.is_some() {
                // Safe because we don't hold any reference to the database currently
                unsafe {
                    updated_iter.del_current()?;
                }
            }
            drop(updated_iter);

            tracing::debug!("write the metadata...");
            let metadata = Metadata {
                dimensions: self.dimensions.try_into().unwrap(),
                items: item_indices,
                roots: ItemIds::from_slice(&roots),
                distance: D::name(),
            };
            self.database.remap_data_type::<MetadataCodec>().put(
                wtxn,
                &Key::metadata(self.index),
                &metadata,
            )?;

            tracing::debug!("write the version...");
            let version = Version {
                major: env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap(),
                minor: env!("CARGO_PKG_VERSION_MINOR").parse().unwrap(),
                patch: env!("CARGO_PKG_VERSION_PATCH").parse().unwrap(),
            };
            self.database.remap_data_type::<VersionCodec>().put(
                wtxn,
                &Key::version(self.index),
                &version,
            )?;

            return Ok(());
        }

        tracing::debug!("reset and retrieve the updated items...");
        (options.progress)(WriterProgress { main: MainStep::RetrieveTheUpdatedItems, sub: None });
        let mut updated_items = RoaringBitmap::new();
        let mut updated_iter = self
            .database
            .remap_types::<PrefixCodec, DecodeIgnore>()
            .prefix_iter_mut(wtxn, &Prefix::updated(self.index))?
            .remap_key_type::<KeyCodec>();
        while let Some((key, _)) = updated_iter.next().transpose()? {
            let inserted = updated_items.push(key.node.item);
            debug_assert!(inserted, "The keys should be sorted by LMDB");
            // Safe because we don't hold any reference to the database currently
            unsafe {
                updated_iter.del_current()?;
            }
        }
        drop(updated_iter);

        // while iterating on the nodes we want to delete all the modified element even if they are being inserted right after.
        let to_delete = &updated_items;
        let to_insert = &item_indices & &updated_items;

        let metadata = self
            .database
            .remap_data_type::<MetadataCodec>()
            .get(wtxn, &Key::metadata(self.index))?;
        let mut roots =
            metadata.as_ref().map_or_else(Vec::new, |metadata| metadata.roots.iter().collect());

        tracing::debug!("Getting a reference to your {} items...", n_items);

        if (options.cancel)() {
            return Err(Error::BuildCancelled);
        }

        let used_node_ids = self.used_tree_node(wtxn)?;
        let nb_tree_nodes = used_node_ids.len();

        let (sub, nodes_extraction_progress) =
            SubStep::new("nodes", item_indices.len() as u32 + nb_tree_nodes as u32);
        (options.progress)(WriterProgress {
            main: MainStep::RetrievingTheTreeAndItemNodes,
            sub: Some(sub),
        });

        let concurrent_node_ids = ConcurrentNodeIds::new(used_node_ids);
        let frozzen_reader = FrozzenReader {
            leafs: &ImmutableLeafs::new(
                wtxn,
                self.database,
                self.index,
                item_indices.len(),
                &nodes_extraction_progress,
            )?,
            trees: &ImmutableTrees::new(
                wtxn,
                self.database,
                self.index,
                nb_tree_nodes,
                &nodes_extraction_progress,
            )?,
            // The globally incrementing node ids that are shared between threads.
            concurrent_node_ids: &concurrent_node_ids,
        };
        let two_means_candidates =
            frozzen_reader.leafs.sample(options.available_memory.unwrap_or(usize::MAX), rng);

        let mut nodes_to_write = Vec::new();

        // If there is metadata it means that we already have trees and we must update them
        if let Some(ref metadata) = metadata {
            tracing::debug!(
                "started inserting new items {} in {} trees...",
                n_items,
                metadata.roots.len()
            );
            let (sub, trees_progress) = SubStep::new("trees", metadata.roots.len() as u32);
            (options.progress)(WriterProgress { main: MainStep::UpdatingTheTrees, sub: Some(sub) });
            let (new_roots, mut tmp_nodes_reader) = self.update_trees(
                options,
                rng,
                metadata,
                &to_insert,
                to_delete,
                &two_means_candidates,
                &frozzen_reader,
                &trees_progress,
            )?;
            nodes_to_write.append(&mut tmp_nodes_reader);
            roots = new_roots;
        }

        tracing::debug!("started building trees for {} items...", n_items);
        tracing::debug!(
            "running {} parallel tree building...",
            options.n_trees.map_or_else(|| "an unknown number of".to_string(), |n| n.to_string())
        );
        // Once we updated the current trees we also need to create the new missing trees
        // So we can run the normal path of building trees from scratch.
        let n_trees_to_build = options
            .n_trees
            .zip(metadata)
            .map(|(n_trees, metadata)| n_trees.saturating_sub(metadata.roots.len()))
            .or(options.n_trees);

        // If we don't have both the metadata and the number of tree to build specified we can't create the substep.
        // But to make the code easier to read, we're going to update an atomic in `build_trees` even if it's not linked to anything.
        let trees_progress = Arc::new(AtomicU32::new(0));
        let sub = n_trees_to_build.map(|trees_to_build| SubStep {
            unit: "trees",
            current: trees_progress.clone(),
            max: trees_to_build as u32,
        });
        (options.progress)(WriterProgress { main: MainStep::CreateNewTrees, sub });

        let (mut thread_roots, mut tmp_nodes) = self.build_trees(
            options,
            rng,
            n_trees_to_build,
            &item_indices,
            &two_means_candidates,
            &frozzen_reader,
            &trees_progress,
        )?;
        nodes_to_write.append(&mut tmp_nodes);

        tracing::debug!("started updating the tree nodes of {} trees...", tmp_nodes.len());
        let (sub, nodes_written_progress) =
            SubStep::new("nodes written", nodes_to_write.len() as u32);
        (options.progress)(WriterProgress {
            main: MainStep::WritingNodesToDatabase,
            sub: Some(sub),
        });
        for (i, tmp_node) in nodes_to_write.iter().enumerate() {
            tracing::debug!(
                "started deleting the {} tree nodes of the {i}nth trees...",
                tmp_node.len()
            );
            for item_id in tmp_node.to_delete() {
                let key = Key::tree(self.index, item_id);
                self.database.remap_data_type::<Bytes>().delete(wtxn, &key)?;
            }
            tracing::debug!(
                "started inserting the {} tree nodes of the {i}nth trees...",
                tmp_node.len()
            );
            for (item_id, item_bytes) in tmp_node.to_insert() {
                let key = Key::tree(self.index, item_id);
                self.database.remap_data_type::<Bytes>().put(wtxn, &key, item_bytes)?;
            }

            nodes_written_progress.fetch_add(1, Ordering::Relaxed);
        }

        if thread_roots.is_empty() {
            // we may have too many nodes
            tracing::debug!("Deleting the extraneous trees if there is some...");
            (options.progress)(WriterProgress { main: MainStep::DeleteExtraneousTrees, sub: None });
            self.delete_extra_trees(
                wtxn,
                options,
                &mut roots,
                options.n_trees,
                concurrent_node_ids.used(),
                n_items,
            )?;
        } else {
            roots.append(&mut thread_roots);
        }

        tracing::debug!("write the metadata...");
        (options.progress)(WriterProgress { main: MainStep::WriteTheMetadata, sub: None });
        let metadata = Metadata {
            dimensions: self.dimensions.try_into().unwrap(),
            items: item_indices,
            roots: ItemIds::from_slice(&roots),
            distance: D::name(),
        };
        match self.database.remap_data_type::<MetadataCodec>().put(
            wtxn,
            &Key::metadata(self.index),
            &metadata,
        ) {
            Ok(_) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update_trees<R: Rng + SeedableRng>(
        &self,
        opt: &BuildOption,
        rng: &mut R,
        metadata: &Metadata,
        to_insert: &RoaringBitmap,
        to_delete: &RoaringBitmap,
        two_means_candidates: &RoaringBitmap,
        frozen_reader: &FrozzenReader<D>,
        trees_progress: &AtomicU32,
    ) -> Result<(Vec<ItemId>, Vec<TmpNodesReader>)> {
        let roots: Vec<_> = metadata.roots.iter().collect();

        repeatn(rng.next_u64(), metadata.roots.len())
            .zip(roots)
            .map(|(seed, root)| {
                tracing::debug!("started updating tree {root:X}...");
                let mut rng = R::seed_from_u64(seed.wrapping_add(root as u64));
                let mut tmp_nodes: TmpNodes<NodeCodec<D>> = match self.tmpdir.as_ref() {
                    Some(path) => TmpNodes::new_in(path)?,
                    None => TmpNodes::new()?,
                };
                let root_node = NodeId::tree(root);
                let (node_id, _items) = self.update_nodes_in_file(
                    opt,
                    frozen_reader,
                    &mut rng,
                    root_node,
                    to_insert,
                    to_delete,
                    two_means_candidates,
                    &mut tmp_nodes,
                )?;
                assert!(node_id.mode != NodeMode::Item, "update_nodes_in_file returned an item even though there was more than a single element");

                trees_progress.fetch_add(1, Ordering::Relaxed);

                tracing::debug!("finished updating tree {root:X}");
                Ok((node_id.unwrap_tree(), tmp_nodes.into_bytes_reader()?))
            })
            .collect()
    }

    /// Update the nodes that changed and delete the deleted nodes all at once.
    /// Run in O(n) on the total number of nodes. Return a tuple containing the
    /// node ID you should use instead of the current_node and the number of
    /// items in the subtree.
    #[allow(clippy::too_many_arguments)]
    fn update_nodes_in_file<R: Rng>(
        &self,
        opt: &BuildOption,
        frozen_reader: &FrozzenReader<D>,
        rng: &mut R,
        current_node: NodeId,
        to_insert: &RoaringBitmap,
        to_delete: &RoaringBitmap,
        two_means_candidates: &RoaringBitmap,
        tmp_nodes: &mut TmpNodes<NodeCodec<D>>,
    ) -> Result<(NodeId, RoaringBitmap)> {
        if (opt.cancel)() {
            return Err(Error::BuildCancelled);
        }
        match current_node.mode {
            NodeMode::Item => {
                // We were called on a specific item, we should create a descendants node
                let mut new_items = RoaringBitmap::from_iter([current_node.item]);
                new_items -= to_delete;
                new_items |= to_insert;

                if new_items.len() == 1 {
                    let item_id = new_items.iter().next().unwrap();
                    if item_id == current_node.item {
                        Ok((current_node, new_items))
                    } else {
                        Ok((NodeId::item(item_id), new_items))
                    }
                } else if self.fit_in_descendant(opt, new_items.len()) {
                    let node_id = frozen_reader.concurrent_node_ids.next()?;
                    let node_id = NodeId::tree(node_id);
                    tmp_nodes.put(
                        node_id.item,
                        &Node::Descendants(Descendants {
                            descendants: Cow::Owned(new_items.clone()),
                        }),
                    )?;
                    Ok((node_id, new_items))
                } else {
                    let new_id = self.make_tree_in_file(
                        opt,
                        frozen_reader,
                        rng,
                        &new_items,
                        two_means_candidates,
                        tmp_nodes,
                    )?;

                    return Ok((new_id, new_items));
                }
            }
            NodeMode::Tree => {
                match frozen_reader.trees.get(current_node.item)?.unwrap() {
                    Node::Leaf(_) => unreachable!(),
                    Node::Descendants(Descendants { descendants }) => {
                        let mut new_descendants = descendants.clone().into_owned();
                        // remove all the deleted IDs before inserting the new elements.
                        new_descendants -= to_delete;

                        // insert all of our IDs in the descendants
                        new_descendants |= to_insert;

                        if descendants.as_ref() == &new_descendants {
                            // if nothing changed, do nothing
                            Ok((current_node, descendants.into_owned()))
                        } else if !self.fit_in_descendant(opt, new_descendants.len()) {
                            // if it doesn't fit in one descendent we need to craft a new whole subtree
                            tmp_nodes.remove(current_node.item);
                            let new_id = self.make_tree_in_file(
                                opt,
                                frozen_reader,
                                rng,
                                &new_descendants,
                                two_means_candidates,
                                tmp_nodes,
                            )?;

                            Ok((new_id, new_descendants))
                        } else if new_descendants.len() == 1 {
                            tmp_nodes.remove(current_node.item);
                            let item = new_descendants.iter().next().unwrap();
                            Ok((NodeId::item(item), new_descendants))
                        } else {
                            // otherwise we can just update our descendants
                            tmp_nodes.put(
                                current_node.item,
                                &Node::Descendants(Descendants {
                                    descendants: Cow::Owned(new_descendants.clone()),
                                }),
                            )?;
                            Ok((current_node, new_descendants))
                        }
                    }
                    Node::SplitPlaneNormal(SplitPlaneNormal { normal, left, right }) => {
                        // Split the to_insert into two bitmaps on the left and right of this normal
                        let mut left_ids = RoaringBitmap::new();
                        let mut right_ids = RoaringBitmap::new();

                        if normal.is_zero() {
                            randomly_split_children(rng, to_insert, &mut left_ids, &mut right_ids);
                        } else {
                            for leaf in to_insert {
                                let node = frozen_reader.leafs.get(leaf)?.unwrap();
                                match D::side(&normal, &node, rng) {
                                    Side::Left => left_ids.insert(leaf),
                                    Side::Right => right_ids.insert(leaf),
                                };
                            }
                        }

                        let (new_left, left_items) = self.update_nodes_in_file(
                            opt,
                            frozen_reader,
                            rng,
                            left,
                            &left_ids,
                            to_delete,
                            two_means_candidates,
                            tmp_nodes,
                        )?;
                        let (new_right, right_items) = self.update_nodes_in_file(
                            opt,
                            frozen_reader,
                            rng,
                            right,
                            &right_ids,
                            to_delete,
                            two_means_candidates,
                            tmp_nodes,
                        )?;

                        let total_items = left_items | right_items;

                        if self.fit_in_descendant(opt, total_items.len()) {
                            // Since we're shrinking we KNOW that new_left and new_right are descendants
                            // thus we can delete them directly knowing there is no sub-tree to look at.
                            if new_left.mode == NodeMode::Tree {
                                tmp_nodes.remove(new_left.item);
                            }
                            if new_right.mode == NodeMode::Tree {
                                tmp_nodes.remove(new_right.item);
                            }

                            tmp_nodes.put(
                                current_node.item,
                                &Node::Descendants(Descendants {
                                    descendants: Cow::Owned(total_items.clone()),
                                }),
                            )?;

                            // we should merge both branch and update ourselves to be a single descendant node
                            Ok((current_node, total_items))
                        } else {
                            // if either the left or the right changed we must update ourselves inplace
                            if new_left != left || new_right != right {
                                tmp_nodes.put(
                                    current_node.item,
                                    &Node::SplitPlaneNormal(SplitPlaneNormal {
                                        normal,
                                        left: new_left,
                                        right: new_right,
                                    }),
                                )?;
                            }

                            // TODO: Should we update the normals if something changed?

                            Ok((current_node, total_items))
                        }
                    }
                }
            }
            NodeMode::Metadata => unreachable!(),
            NodeMode::Updated => todo!(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_trees<R: Rng + SeedableRng>(
        &self,
        opt: &BuildOption,
        rng: &mut R,
        n_trees: Option<usize>,
        item_indices: &RoaringBitmap,
        two_means_candidates: &RoaringBitmap,
        frozen_reader: &FrozzenReader<D>,
        progress: &AtomicU32,
    ) -> Result<(Vec<ItemId>, Vec<TmpNodesReader>)> {
        let n_items = item_indices.len();
        let concurrent_node_ids = frozen_reader.concurrent_node_ids;

        repeatn(rng.next_u64(), n_trees.unwrap_or(usize::MAX))
            .enumerate()
            // Stop generating trees once the specified number of tree nodes are generated
            // but continue to generate trees if the number of trees is unspecified
            .take_any_while(|_| match n_trees {
                Some(_) => true,
                None => concurrent_node_ids.used() < n_items,
            })
            .map(|(i, seed)| {
                tracing::debug!("started generating tree {i:X}...");
                let mut rng = R::seed_from_u64(seed.wrapping_add(i as u64));
                let mut tmp_nodes = match self.tmpdir.as_ref() {
                    Some(path) => TmpNodes::new_in(path)?,
                    None => TmpNodes::new()?,
                };
                let root_id =
                    self.make_tree_in_file(opt, frozen_reader, &mut rng, item_indices, two_means_candidates, &mut tmp_nodes)?;
                assert!(
                    root_id.mode != NodeMode::Item,
                    "make_tree_in_file returned an item even though there was more than a single element"
                );
                tracing::debug!("finished generating tree {i:X}");
                progress.fetch_add(1, Ordering::Relaxed);

                // make_tree will NEVER return a leaf when called as root
                Ok((root_id.unwrap_tree(), tmp_nodes.into_bytes_reader()?))
            })
            .collect()
    }

    /// Creates a tree of nodes from the frozzen items that lives
    /// in the database and generates descendants, split normal
    /// and root nodes in files that will be stored in the database later.
    fn make_tree_in_file<R: Rng>(
        &self,
        opt: &BuildOption,
        reader: &FrozzenReader<D>,
        rng: &mut R,
        item_indices: &RoaringBitmap,
        two_means_candidates: &RoaringBitmap,
        tmp_nodes: &mut TmpNodes<NodeCodec<D>>,
    ) -> Result<NodeId> {
        if (opt.cancel)() {
            return Err(Error::BuildCancelled);
        }
        if item_indices.len() == 1 {
            return Ok(NodeId::item(item_indices.min().unwrap()));
        }

        if self.fit_in_descendant(opt, item_indices.len()) {
            let item_id = reader.concurrent_node_ids.next()?;
            let item = Node::Descendants(Descendants { descendants: Cow::Borrowed(item_indices) });
            tmp_nodes.put(item_id, &item)?;
            return Ok(NodeId::tree(item_id));
        }

        // If we don't have at least two vectors in common between the item_indices and the two_means_candidates
        // we're going to use the whole list of two_means_candidates to create the split node.
        let intersection_len = two_means_candidates.intersection_len(item_indices);
        let two_means_candidates = if intersection_len >= 2 {
            &(two_means_candidates & item_indices)
        } else {
            two_means_candidates
        };

        let children = ImmutableSubsetLeafs::from_item_ids(reader.leafs, two_means_candidates);
        let mut children_left = Vec::with_capacity(children.len() as usize);
        let mut children_right = Vec::with_capacity(children.len() as usize);
        let mut remaining_attempts = 3;

        let mut normal = loop {
            children_left.clear();
            children_right.clear();

            let normal = D::create_split(&children, rng)?;
            for item_id in item_indices.iter() {
                let node = reader.leafs.get(item_id)?.unwrap();
                match D::side(&normal, &node, rng) {
                    Side::Left => children_left.push(item_id),
                    Side::Right => children_right.push(item_id),
                };
            }

            if split_imbalance(children_left.len() as u64, children_right.len() as u64) < 0.95
                || remaining_attempts == 0
            {
                break normal;
            }

            remaining_attempts -= 1;
        };

        // If we didn't find a hyperplane, just randomize sides as a last option
        // and set the split plane to zero as a dummy plane.
        let (children_left, children_right) =
            if split_imbalance(children_left.len() as u64, children_right.len() as u64) > 0.99 {
                let mut children_left = RoaringBitmap::new();
                let mut children_right = RoaringBitmap::new();
                randomly_split_children(rng, item_indices, &mut children_left, &mut children_right);
                UnalignedVector::reset(&mut normal);

                (children_left, children_right)
            } else {
                (
                    RoaringBitmap::from_sorted_iter(children_left).unwrap(),
                    RoaringBitmap::from_sorted_iter(children_right).unwrap(),
                )
            };

        let normal = SplitPlaneNormal {
            normal,
            left: self.make_tree_in_file(
                opt,
                reader,
                rng,
                &children_left,
                two_means_candidates,
                tmp_nodes,
            )?,
            right: self.make_tree_in_file(
                opt,
                reader,
                rng,
                &children_right,
                two_means_candidates,
                tmp_nodes,
            )?,
        };

        let new_node_id = reader.concurrent_node_ids.next()?;
        tmp_nodes.put(new_node_id, &Node::SplitPlaneNormal(normal))?;

        Ok(NodeId::tree(new_node_id))
    }

    /// Delete any extraneous trees.
    fn delete_extra_trees(
        &self,
        wtxn: &mut RwTxn,
        opt: &BuildOption,
        roots: &mut Vec<ItemId>,
        nb_trees: Option<usize>,
        nb_tree_nodes: u64,
        nb_items: u64,
    ) -> Result<()> {
        if roots.is_empty() {
            return Ok(());
        }
        let nb_trees = match nb_trees {
            Some(nb_trees) => nb_trees,
            None => {
                // 1. Estimate the number of nodes per tree; the division is safe because we ensured there was at least one root node above.
                let nodes_per_tree = nb_tree_nodes / roots.len() as u64;
                // 2. Estimate the number of tree we need to have AT LEAST as much tree-nodes than items
                (nb_items / nodes_per_tree) as usize
            }
        };

        if roots.len() > nb_trees {
            // we have too many trees and must delete some of them
            let to_delete = roots.len() - nb_trees;

            // we want to delete the oldest tree first since they're probably
            // the less precise one
            let new_roots = roots.split_off(to_delete);
            let to_delete = mem::replace(roots, new_roots);
            tracing::debug!("Deleting {} trees", to_delete.len());

            for tree in to_delete {
                if (opt.cancel)() {
                    return Err(Error::BuildCancelled);
                }
                self.delete_tree(wtxn, NodeId::tree(tree))?;
            }
        }

        Ok(())
    }

    fn delete_tree(&self, wtxn: &mut RwTxn, node: NodeId) -> Result<()> {
        let key = Key::new(self.index, node);
        match self.database.get(wtxn, &key)?.ok_or(Error::missing_key(key))? {
            // the leafs are shared between the trees, we MUST NOT delete them.
            Node::Leaf(_) => Ok(()),
            Node::Descendants(_) => {
                self.database.delete(wtxn, &key).map(|_| ()).map_err(Error::from)
            }
            Node::SplitPlaneNormal(SplitPlaneNormal { normal: _, left, right }) => {
                self.delete_tree(wtxn, left)?;
                self.delete_tree(wtxn, right)?;
                self.database.delete(wtxn, &key).map(|_| ()).map_err(Error::from)
            }
        }
    }

    // Fetches the item's ids, not the tree nodes ones.
    fn item_indices(&self, wtxn: &mut RwTxn<'_>) -> heed::Result<RoaringBitmap> {
        let mut indices = RoaringBitmap::new();
        for result in self
            .database
            .remap_types::<PrefixCodec, DecodeIgnore>()
            .prefix_iter(wtxn, &Prefix::item(self.index))?
            .remap_key_type::<KeyCodec>()
        {
            let (i, _) = result?;
            indices.push(i.node.unwrap_item());
        }

        Ok(indices)
    }
}

/// Represents the final version of the leafs and contains
/// useful informations to synchronize the building threads.
#[derive(Clone)]
struct FrozzenReader<'a, D: Distance> {
    leafs: &'a ImmutableLeafs<'a, D>,
    trees: &'a ImmutableTrees<'a, D>,
    concurrent_node_ids: &'a ConcurrentNodeIds,
}

/// Randomly and efficiently splits the items into the left and right children vectors.
fn randomly_split_children<R: Rng>(
    rng: &mut R,
    item_indices: &RoaringBitmap,
    children_left: &mut RoaringBitmap,
    children_right: &mut RoaringBitmap,
) {
    children_left.clear();
    children_right.clear();

    // Split it in half and put the right half into the right children's vector
    for item_id in item_indices {
        match Side::random(rng) {
            Side::Left => children_left.push(item_id),
            Side::Right => children_right.push(item_id),
        };
    }
}

/// Clears everything but the leafs nodes (items).
/// Starts from the last node and stops at the first leaf.
fn clear_tree_nodes<D: Distance>(
    wtxn: &mut RwTxn,
    database: Database<D>,
    index: u16,
) -> Result<()> {
    database.delete(wtxn, &Key::metadata(index))?;
    let mut cursor = database
        .remap_types::<PrefixCodec, DecodeIgnore>()
        .prefix_iter_mut(wtxn, &Prefix::tree(index))?
        .remap_key_type::<DecodeIgnore>();
    while let Some((_id, _node)) = cursor.next().transpose()? {
        // safety: we keep no reference into the database between operations
        unsafe { cursor.del_current()? };
    }

    Ok(())
}

fn split_imbalance(left_indices_len: u64, right_indices_len: u64) -> f64 {
    let ls = left_indices_len as f64;
    let rs = right_indices_len as f64;
    let f = ls / (ls + rs + f64::EPSILON); // Avoid 0/0
    f.max(1.0 - f)
}
