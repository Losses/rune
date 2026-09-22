use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::iter::repeat;
use std::marker;
use std::num::NonZeroUsize;

use heed::types::DecodeIgnore;
use heed::RoTxn;
use ordered_float::OrderedFloat;
use roaring::RoaringBitmap;

use crate::distance::Distance;
use crate::internals::{KeyCodec, Side};
use crate::item_iter::ItemIter;
use crate::node::{Descendants, ItemIds, Leaf, SplitPlaneNormal};
use crate::unaligned_vector::UnalignedVector;
use crate::{
    Database, Error, ItemId, Key, MetadataCodec, Node, NodeId, Prefix, PrefixCodec, Result, Stats,
    TreeStats,
};

/// Options used to make a query against an arroy [`Reader`].
pub struct QueryBuilder<'a, D: Distance> {
    reader: &'a Reader<'a, D>,
    count: usize,
    search_k: Option<NonZeroUsize>,
    oversampling: Option<NonZeroUsize>,
    candidates: Option<&'a RoaringBitmap>,
}

impl<'a, D: Distance> QueryBuilder<'a, D> {
    /// Returns the closests items from `item`.
    ///
    /// See also [`Self::by_vector`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use arroy::{Reader, distances::Euclidean};
    /// # let (reader, rtxn): (Reader<Euclidean>, heed::RoTxn) = todo!();
    /// reader.nns(20).by_item(&rtxn, 5);
    /// ```
    pub fn by_item(&self, rtxn: &RoTxn, item: ItemId) -> Result<Option<Vec<(ItemId, f32)>>> {
        match item_leaf(self.reader.database, self.reader.index, rtxn, item)? {
            Some(leaf) => self.reader.nns_by_leaf(rtxn, &leaf, self).map(Some),
            None => Ok(None),
        }
    }

    /// Returns the closest items from the provided `vector`.
    ///
    /// See also [`Self::by_item`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use arroy::{Reader, distances::Euclidean};
    /// # let (reader, rtxn): (Reader<Euclidean>, heed::RoTxn) = todo!();
    /// reader.nns(20).by_vector(&rtxn, &[1.25854, -0.75598, 0.58524]);
    /// ```
    pub fn by_vector(&self, rtxn: &RoTxn, vector: &'a [f32]) -> Result<Vec<(ItemId, f32)>> {
        if vector.len() != self.reader.dimensions() {
            return Err(Error::InvalidVecDimension {
                expected: self.reader.dimensions(),
                received: vector.len(),
            });
        }

        let vector = UnalignedVector::from_slice(vector);
        let leaf = Leaf { header: D::new_header(&vector), vector };
        self.reader.nns_by_leaf(rtxn, &leaf, self)
    }

    /// During the query, arroy will inspect up to `search_k` nodes which defaults
    /// to `n_trees * count` if not provided. `search_k` gives you a run-time
    /// tradeoff between better accuracy and speed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use arroy::{Reader, distances::Euclidean};
    /// # let (reader, rtxn): (Reader<Euclidean>, heed::RoTxn) = todo!();
    /// use std::num::NonZeroUsize;
    /// reader.nns(20).search_k(NonZeroUsize::new(1000).unwrap()).by_item(&rtxn, 3);
    /// ```
    pub fn search_k(&mut self, search_k: NonZeroUsize) -> &mut Self {
        self.search_k = Some(search_k);
        self
    }

    /// Oversampling will multiply [`QueryBuilder::search_k`] by the specified number.
    /// That's useful when you don't want to compute `search_k` yourself.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use arroy::{Reader, distances::Euclidean};
    /// # let (reader, rtxn): (Reader<Euclidean>, heed::RoTxn) = todo!();
    /// use std::num::NonZeroUsize;
    /// reader.nns(20).oversampling(NonZeroUsize::new(6).unwrap()).by_item(&rtxn, 5);
    /// ```
    pub fn oversampling(&mut self, oversampling: NonZeroUsize) -> &mut Self {
        self.oversampling = Some(oversampling);
        self
    }

    /// Specify a subset of candidates to inspect. Filters out everything else.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use arroy::{Reader, distances::Euclidean};
    /// # let (reader, rtxn): (Reader<Euclidean>, heed::RoTxn) = todo!();
    /// let candidates = roaring::RoaringBitmap::from_iter([1, 3, 4, 5, 6, 7, 8, 9, 15, 16]);
    /// reader.nns(20).candidates(&candidates).by_item(&rtxn, 6);
    /// ```
    pub fn candidates(&mut self, candidates: &'a RoaringBitmap) -> &mut Self {
        self.candidates = Some(candidates);
        self
    }
}

/// A reader over the arroy trees and user items.
#[derive(Debug)]
pub struct Reader<'t, D: Distance> {
    database: Database<D>,
    index: u16,
    roots: ItemIds<'t>,
    dimensions: usize,
    items: RoaringBitmap,
    _marker: marker::PhantomData<D>,
}

impl<'t, D: Distance> Reader<'t, D> {
    /// Returns a reader over the database with the specified [`Distance`] type.
    pub fn open(rtxn: &'t RoTxn, index: u16, database: Database<D>) -> Result<Reader<'t, D>> {
        let metadata_key = Key::metadata(index);
        let metadata = match database.remap_data_type::<MetadataCodec>().get(rtxn, &metadata_key)? {
            Some(metadata) => metadata,
            None => return Err(Error::MissingMetadata(index)),
        };

        if D::name() != metadata.distance {
            return Err(Error::UnmatchingDistance {
                expected: metadata.distance.to_owned(),
                received: D::name(),
            });
        }
        if database
            .remap_types::<PrefixCodec, DecodeIgnore>()
            .prefix_iter(rtxn, &Prefix::updated(index))?
            .remap_key_type::<KeyCodec>()
            .next()
            .is_some()
        {
            return Err(Error::NeedBuild(index));
        }

        Ok(Reader {
            database: database.remap_data_type(),
            index,
            roots: metadata.roots,
            dimensions: metadata.dimensions.try_into().unwrap(),
            items: metadata.items,
            _marker: marker::PhantomData,
        })
    }

    /// Returns the number of dimensions in the index.
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Returns the number of trees in the index.
    pub fn n_trees(&self) -> usize {
        self.roots.len()
    }

    /// Returns the number of vectors stored in the index.
    pub fn n_items(&self) -> u64 {
        self.items.len()
    }

    /// Returns all the item ids contained in this index.
    pub fn item_ids(&self) -> &RoaringBitmap {
        &self.items
    }

    /// Returns the index of this reader in the database.
    pub fn index(&self) -> u16 {
        self.index
    }

    /// Returns the stats of the trees of this database.
    pub fn stats(&self, rtxn: &RoTxn) -> Result<Stats> {
        fn recursive_depth<D: Distance>(
            rtxn: &RoTxn,
            database: Database<D>,
            index: u16,
            node_id: NodeId,
        ) -> Result<TreeStats> {
            match database.get(rtxn, &Key::new(index, node_id))?.unwrap() {
                Node::Leaf(_) => {
                    Ok(TreeStats { depth: 1, dummy_normals: 0, split_nodes: 0, descendants: 0 })
                }
                Node::Descendants(_) => {
                    Ok(TreeStats { depth: 1, dummy_normals: 0, split_nodes: 0, descendants: 1 })
                }
                Node::SplitPlaneNormal(SplitPlaneNormal { normal, left, right }) => {
                    let left = recursive_depth(rtxn, database, index, left)?;
                    let right = recursive_depth(rtxn, database, index, right)?;
                    let is_zero_normal = normal.is_zero() as usize;

                    Ok(TreeStats {
                        depth: 1 + left.depth.max(right.depth),
                        dummy_normals: left.dummy_normals + right.dummy_normals + is_zero_normal,
                        split_nodes: left.split_nodes + right.split_nodes + 1,
                        descendants: left.descendants + right.descendants,
                    })
                }
            }
        }

        let tree_stats: Result<Vec<_>> = self
            .roots
            .iter()
            .map(NodeId::tree)
            .map(|root| recursive_depth::<D>(rtxn, self.database, self.index, root))
            .collect();

        Ok(Stats { tree_stats: tree_stats?, leaf: self.items.len() })
    }

    /// Returns the number of nodes in the index. Useful to run an exhaustive search.
    pub fn n_nodes(&self, rtxn: &'t RoTxn) -> Result<Option<NonZeroUsize>> {
        Ok(NonZeroUsize::new(self.database.len(rtxn)? as usize))
    }

    /// Returns the vector for item `i` that was previously added.
    pub fn item_vector(&self, rtxn: &'t RoTxn, item: ItemId) -> Result<Option<Vec<f32>>> {
        Ok(item_leaf(self.database, self.index, rtxn, item)?.map(|leaf| {
            let mut vec = leaf.vector.to_vec();
            vec.truncate(self.dimensions());
            vec
        }))
    }

    /// Returns `true` if the index is empty.
    pub fn is_empty(&self, rtxn: &RoTxn) -> Result<bool> {
        self.iter(rtxn).map(|mut iter| iter.next().is_none())
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
    pub fn iter(&self, rtxn: &'t RoTxn) -> Result<ItemIter<'t, D>> {
        Ok(ItemIter {
            inner: self
                .database
                .remap_key_type::<PrefixCodec>()
                .prefix_iter(rtxn, &Prefix::item(self.index))?
                .remap_key_type::<KeyCodec>(),
        })
    }

    /// Return a [`QueryBuilder`] that lets you configure and execute a search request.
    ///
    /// You must provide the number of items you want to receive.
    pub fn nns(&self, count: usize) -> QueryBuilder<'_, D> {
        QueryBuilder { reader: self, count, search_k: None, oversampling: None, candidates: None }
    }

    fn nns_by_leaf(
        &self,
        rtxn: &'t RoTxn,
        query_leaf: &Leaf<D>,
        opt: &QueryBuilder<D>,
    ) -> Result<Vec<(ItemId, f32)>> {
        if self.items.is_empty() {
            return Ok(Vec::new());
        }
        let candidates = opt.candidates.map(|candidates| candidates & &self.items);

        let nns = match candidates {
            // When we're filtering on 5% or less of the database we don't use the trees and
            // just sort every candidates by hand
            Some(candidates) if (candidates.len() as f32 / self.items.len() as f32) < 0.5 => {
                candidates.iter().collect()
            }
            _ => {
                // Since the datastructure describes a kind of btree, the capacity is something in the order of:
                // The number of root nodes + log2 of the total number of vectors.
                let mut queue =
                    BinaryHeap::with_capacity(self.roots.len() + self.items.len().ilog2() as usize);
                let search_k = opt.search_k.map_or(opt.count * self.roots.len(), NonZeroUsize::get);
                let search_k = opt
                    .oversampling
                    .map_or(search_k.saturating_mul(D::DEFAULT_OVERSAMPLING), |oversampling| {
                        search_k.saturating_mul(oversampling.get())
                    });

                // Insert all the root nodes and associate them to the highest distance.
                queue.extend(
                    repeat(OrderedFloat(f32::INFINITY)).zip(self.roots.iter().map(NodeId::tree)),
                );

                let mut nns = Vec::new();
                while nns.len() < search_k {
                    let (OrderedFloat(dist), item) = match queue.pop() {
                        Some(out) => out,
                        None => break,
                    };

                    let key = Key::new(self.index, item);
                    match self.database.get(rtxn, &key)?.ok_or(Error::missing_key(key))? {
                        Node::Leaf(_) => {
                            if opt.candidates.map_or(true, |c| c.contains(item.item)) {
                                nns.push(item.unwrap_item());
                            }
                        }
                        Node::Descendants(Descendants { descendants }) => {
                            if let Some(candidates) = opt.candidates {
                                nns.extend((descendants.into_owned() & candidates).iter());
                            } else {
                                nns.extend(descendants.iter());
                            }
                        }
                        Node::SplitPlaneNormal(SplitPlaneNormal { normal, left, right }) => {
                            let margin = D::margin_no_header(&normal, &query_leaf.vector);
                            queue.push((
                                OrderedFloat(D::pq_distance(dist, margin, Side::Left)),
                                left,
                            ));
                            queue.push((
                                OrderedFloat(D::pq_distance(dist, margin, Side::Right)),
                                right,
                            ));
                        }
                    }
                }

                // Get distances for all items
                // To avoid calculating distance multiple times for any items, sort by id and dedup by id.
                nns.sort_unstable();
                nns.dedup();
                nns
            }
        };

        let mut nns_distances = Vec::with_capacity(nns.len());
        for nn in nns {
            let key = Key::item(self.index, nn);
            let leaf = match self.database.get(rtxn, &key)?.ok_or(Error::missing_key(key))? {
                Node::Leaf(leaf) => leaf,
                Node::Descendants(_) | Node::SplitPlaneNormal(_) => unreachable!(),
            };
            let distance = D::built_distance(query_leaf, &leaf);
            nns_distances.push(Reverse((OrderedFloat(distance), nn)));
        }

        let mut sorted_nns = BinaryHeap::from(nns_distances);
        let capacity = opt.count.min(sorted_nns.len());
        let mut output = Vec::with_capacity(capacity);
        while let Some(Reverse((OrderedFloat(dist), item))) = sorted_nns.pop() {
            if output.len() == capacity {
                break;
            }
            output.push((item, D::normalized_distance(dist, self.dimensions)));
        }

        Ok(output)
    }

    #[cfg(feature = "plot")]
    /// Write the internal arroy graph in dot format into the provided writer.
    pub fn plot_internals_tree_nodes(
        &self,
        rtxn: &RoTxn,
        mut writer: impl std::io::Write,
    ) -> Result<()> {
        writeln!(writer, "digraph {{")?;
        writeln!(writer, "\tlabel=metadata")?;
        writeln!(writer)?;

        if let Some(tree) = self.roots.iter().next() {
            // subgraph {
            //   a -> b
            //   a -> b
            //   b -> a
            // }

            let mut cache = std::collections::HashMap::<NodeId, u64>::new();

            // Start creating the graph
            writeln!(writer, "\tsubgraph {{")?;
            writeln!(writer, "\t\troot [color=blue]")?;
            writeln!(writer, "\t\troot -> {tree}")?;

            let mut explore = vec![Key::tree(self.index, tree)];
            while let Some(key) = explore.pop() {
                match self.database.get(rtxn, &key)?.unwrap() {
                    Node::Leaf(_) => (),
                    Node::Descendants(Descendants { descendants: _ }) => {
                        writeln!(writer, "\t\t{} [label=\"{}\"]", key.node.item, key.node.item,)?
                    }
                    Node::SplitPlaneNormal(SplitPlaneNormal { normal, left, right }) => {
                        if normal.is_zero() {
                            writeln!(writer, "\t\t{} [color=red]", key.node.item)?;
                        }
                        writeln!(
                            writer,
                            "\t\t{} -> {} [taillabel=\"{}\"]",
                            key.node.item,
                            left.item,
                            self.nb_sub_nodes(rtxn, left, &mut cache)?
                        )?;
                        writeln!(
                            writer,
                            "\t\t{} -> {} [taillabel=\"{}\"]",
                            key.node.item,
                            right.item,
                            self.nb_sub_nodes(rtxn, right, &mut cache)?
                        )?;
                        explore.push(Key::tree(self.index, left.item));
                        explore.push(Key::tree(self.index, right.item));
                    }
                }
            }

            writeln!(writer, "\t}}")?;
        }

        writeln!(writer, "}}")?;

        Ok(())
    }

    #[cfg(feature = "plot")]
    /// Return the number of nodes in a node.
    fn nb_sub_nodes(
        &self,
        rtxn: &RoTxn,
        node_id: NodeId,
        cache: &mut std::collections::HashMap<NodeId, u64>,
    ) -> Result<u64> {
        if let Some(count) = cache.get(&node_id) {
            return Ok(*count);
        }

        match self.database.get(rtxn, &Key::new(self.index, node_id))?.unwrap() {
            Node::Leaf(_) => Ok(1),
            Node::Descendants(Descendants { descendants }) => Ok(descendants.len()),
            Node::SplitPlaneNormal(SplitPlaneNormal { normal: _, left, right }) => {
                let left = self.nb_sub_nodes(rtxn, left, cache)?;
                let right = self.nb_sub_nodes(rtxn, right, cache)?;
                let nb_descendants = left + right;

                cache.insert(node_id, nb_descendants);
                Ok(nb_descendants)
            }
        }
    }

    /// Verify that the whole reader is correctly formed:
    /// - We can access all the items.
    /// - All the tree nodes are part of a tree.
    /// - No tree shares the same tree node.
    /// - We're effectively working with trees and not graphs (i.e., an item or tree node cannot be linked twice in the tree)
    #[cfg(any(test, feature = "assert-reader-validity"))]
    pub fn assert_validity(&self, rtxn: &RoTxn) -> Result<()> {
        // First, get all the items
        let mut item_ids = RoaringBitmap::new();
        for result in self
            .database
            .remap_types::<PrefixCodec, DecodeIgnore>()
            .prefix_iter(rtxn, &Prefix::item(self.index))?
            .remap_key_type::<KeyCodec>()
        {
            let (i, _) = result?;
            item_ids.push(i.node.unwrap_item());
        }
        // Second, get all the tree nodes
        let mut tree_ids = RoaringBitmap::new();
        for result in self
            .database
            .remap_types::<PrefixCodec, DecodeIgnore>()
            .prefix_iter(rtxn, &Prefix::tree(self.index))?
            .remap_key_type::<KeyCodec>()
        {
            let (i, _) = result?;
            tree_ids.push(i.node.unwrap_tree());
        }

        // The get all the items AND tree nodes PER trees
        for root in self.roots.iter() {
            let (trees, items) = self.gather_items_and_tree_ids(rtxn, NodeId::tree(root))?;
            // Ensure that every tree can access all items
            assert_eq!(item_ids, items, "A tree cannot access to all items");
            // We can remove the already explored tree nodes
            assert!(tree_ids.is_superset(&trees), "A tree contains an invalid tree node. Either doesn't exist or was already used in another tree");
            tree_ids -= trees;
        }

        assert!(tree_ids.is_empty(), "There is {tree_ids:?} tree nodes floating around");
        Ok(())
    }

    /// Return first the number of tree nodes and second the items accessible from a node.
    /// And ensure that an item or tree node is never linked twice in the tree
    #[cfg(any(test, feature = "assert-reader-validity"))]
    fn gather_items_and_tree_ids(
        &self,
        rtxn: &RoTxn,
        node_id: NodeId,
    ) -> Result<(RoaringBitmap, RoaringBitmap)> {
        match self.database.get(rtxn, &Key::new(self.index, node_id))?.unwrap() {
            Node::Leaf(_) => Ok((
                RoaringBitmap::new(),
                RoaringBitmap::from_sorted_iter(Some(node_id.item)).unwrap(),
            )),
            Node::Descendants(Descendants { descendants }) => Ok((
                RoaringBitmap::from_sorted_iter(Some(node_id.item)).unwrap(),
                descendants.into_owned(),
            )),
            Node::SplitPlaneNormal(SplitPlaneNormal { normal: _, left, right }) => {
                let left = self.gather_items_and_tree_ids(rtxn, left)?;
                let right = self.gather_items_and_tree_ids(rtxn, right)?;

                let total_trees_size = left.0.len() + right.0.len();
                let total_items_size = left.1.len() + right.1.len();

                let mut trees = left.0 | right.0;
                let items = left.1 | right.1;

                // We should never find the same tree node or item ID in a single tree.
                assert_eq!(total_trees_size, trees.len());
                assert_eq!(total_items_size, items.len());

                trees.insert(node_id.item);

                Ok((trees, items))
            }
        }
    }
}

pub fn item_leaf<'a, D: Distance>(
    database: Database<D>,
    index: u16,
    rtxn: &'a RoTxn,
    item: ItemId,
) -> Result<Option<Leaf<'a, D>>> {
    match database.get(rtxn, &Key::item(index, item))? {
        Some(Node::Leaf(leaf)) => Ok(Some(leaf)),
        Some(Node::SplitPlaneNormal(_)) => Ok(None),
        Some(Node::Descendants(_)) => Ok(None),
        None => Ok(None),
    }
}
