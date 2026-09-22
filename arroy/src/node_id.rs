use std::fmt;
use std::mem::size_of;

use byteorder::{BigEndian, ByteOrder};

use crate::ItemId;

/// /!\ Changing the value of the enum can be DB-breaking /!\
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum NodeMode {
    /// Stores the metadata under the `ItemId` 0
    Metadata = 0,
    /// Stores the list of all the `ItemId` that have been updated.
    /// We only stores `Unit` values under the keys.
    Updated = 1,
    /// The tree nodes are stored under this id.
    Tree = 2,
    /// The original vectors are stored under this id in `Leaf` structures.
    Item = 3,
}

impl TryFrom<u8> for NodeMode {
    type Error = String;

    fn try_from(v: u8) -> std::result::Result<Self, Self::Error> {
        match v {
            v if v == NodeMode::Item as u8 => Ok(NodeMode::Item),
            v if v == NodeMode::Tree as u8 => Ok(NodeMode::Tree),
            v if v == NodeMode::Updated as u8 => Ok(NodeMode::Updated),
            v if v == NodeMode::Metadata as u8 => Ok(NodeMode::Metadata),
            v => Err(format!("Could not convert {v} as a `NodeMode`.")),
        }
    }
}

/// Point to a node in the tree. Can be any kind of node.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    // Indicate what the item represent.
    pub mode: NodeMode,
    /// The item we want to get.
    pub item: ItemId,
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}({})", self.mode, self.item)
    }
}

impl NodeId {
    pub const fn metadata() -> Self {
        Self { mode: NodeMode::Metadata, item: 0 }
    }

    pub const fn version() -> Self {
        Self { mode: NodeMode::Metadata, item: 1 }
    }

    pub const fn updated(item: u32) -> Self {
        Self { mode: NodeMode::Updated, item }
    }

    pub const fn tree(item: u32) -> Self {
        Self { mode: NodeMode::Tree, item }
    }

    pub const fn item(item: u32) -> Self {
        Self { mode: NodeMode::Item, item }
    }

    /// Return the underlying `ItemId` if it is an item.
    /// Panic otherwise.
    #[track_caller]
    pub fn unwrap_item(&self) -> ItemId {
        assert_eq!(self.mode, NodeMode::Item);
        self.item
    }

    /// Return the underlying `ItemId` if it is a tree node.
    /// Panic otherwise.
    #[track_caller]
    pub fn unwrap_tree(&self) -> ItemId {
        assert_eq!(self.mode, NodeMode::Tree);
        self.item
    }

    pub fn to_bytes(self) -> [u8; 5] {
        let mut output = [0; 5];

        output[0] = self.mode as u8;
        let item_bytes = self.item.to_be_bytes();
        output[1..].copy_from_slice(&item_bytes);

        output
    }

    pub fn from_bytes(bytes: &[u8]) -> (Self, &[u8]) {
        let mode = NodeMode::try_from(bytes[0]).expect("Could not parse the node mode");
        let item = BigEndian::read_u32(&bytes[1..]);

        (Self { mode, item }, &bytes[size_of::<NodeMode>() + size_of::<ItemId>()..])
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn check_node_id_ordering() {
        assert!(NodeId::item(0) == NodeId::item(0));
        assert!(NodeId::item(1) > NodeId::item(0));
        assert!(NodeId::item(0) < NodeId::item(1));

        assert!(NodeId::tree(0) == NodeId::tree(0));
        assert!(NodeId::tree(1) > NodeId::tree(0));
        assert!(NodeId::tree(0) < NodeId::tree(1));

        assert!(NodeId::updated(0) == NodeId::updated(0));
        assert!(NodeId::updated(1) > NodeId::updated(0));
        assert!(NodeId::updated(0) < NodeId::updated(1));

        // tree < item whatever is the value
        assert!(NodeId::tree(u32::MAX) < NodeId::item(0));

        assert!(NodeId::metadata() == NodeId::metadata());
        assert!(NodeId::metadata() < NodeId::tree(u32::MIN));
        assert!(NodeId::metadata() < NodeId::updated(u32::MIN));
        assert!(NodeId::metadata() < NodeId::item(u32::MIN));
    }
}
