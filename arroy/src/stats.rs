/// The different stats of an arroy database.
#[derive(Debug, Clone)]
pub struct Stats {
    /// Number of leaf node or item vectors.
    pub leaf: u64,
    /// The stats of each individual tree.
    pub tree_stats: Vec<TreeStats>,
}

/// The different stats of a tree in an arroy database.
#[derive(Debug, Copy, Clone)]
pub struct TreeStats {
    /// The depth of the tree.
    pub depth: usize,
    /// The number of split plane normals that were set to zero
    /// and where children are randomly assigned a side.
    pub dummy_normals: usize,
    /// Number of split nodes in the tree.
    pub split_nodes: usize,
    /// Number of descendants nodes in the tree.
    pub descendants: usize,
}
