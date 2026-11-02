//! Portable, versioned checkpoints for unkeyed BLAKE3's public chunk tree.
//! The on-disk representation is independent of the crate's in-memory structs.
use super::{Digest, MAX_NUMBER, Result, SyncError};
const CHUNK: usize = 1024;
const MAGIC: &[u8; 4] = b"B3P1";
#[derive(Clone, Debug, Default)]
pub struct PortableHasher {
    chunks: u64,
    pending: Vec<u8>,
    stack: Vec<Digest>,
}
impl serde::Serialize for PortableHasher {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        super::bytes_serde::serialize(&self.checkpoint(), serializer)
    }
}
impl<'de> serde::Deserialize<'de> for PortableHasher {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let bytes = super::bytes_serde::deserialize(deserializer)?;
        Self::from_checkpoint(&bytes).map_err(serde::de::Error::custom)
    }
}
impl PortableHasher {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> u64 {
        self.chunks * CHUNK as u64 + self.pending.len() as u64
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn update(&mut self, mut input: &[u8]) -> Result<()> {
        let current = self
            .chunks
            .checked_mul(CHUNK as u64)
            .and_then(|n| n.checked_add(self.pending.len() as u64))
            .ok_or(SyncError::LogicalLimit)?;
        if current
            .checked_add(input.len() as u64)
            .is_none_or(|n| n > MAX_NUMBER)
        {
            return Err(SyncError::LogicalLimit);
        }
        while !input.is_empty() {
            if self.pending.len() == CHUNK {
                let cv = blake3::guts::ChunkState::new(self.chunks)
                    .update(&self.pending)
                    .finalize(false);
                self.chunks += 1;
                let mut n = self.chunks;
                let mut value = cv;
                while n & 1 == 0 {
                    let left = self
                        .stack
                        .pop()
                        .ok_or(SyncError::Invalid("invalid hash stack".into()))?;
                    value = blake3::guts::parent_cv(&blake3::Hash::from_bytes(left), &value, false);
                    n >>= 1;
                }
                self.stack.push(*value.as_bytes());
                self.pending.clear();
            }
            let take = (CHUNK - self.pending.len()).min(input.len());
            self.pending.extend_from_slice(&input[..take]);
            input = &input[take..];
        }
        Ok(())
    }
    pub fn finish(&self) -> Digest {
        let mut chunk = blake3::guts::ChunkState::new(self.chunks);
        chunk.update(&self.pending);
        if self.stack.is_empty() {
            return *chunk.finalize(true).as_bytes();
        }
        let mut value = chunk.finalize(false);
        for (i, left) in self.stack.iter().enumerate().rev() {
            value = blake3::guts::parent_cv(&blake3::Hash::from_bytes(*left), &value, i == 0);
        }
        *value.as_bytes()
    }
    /// B3P1, big-endian completed chunk count, pending length, stack length,
    /// pending bytes, then 32-byte chaining values from oldest to newest.
    pub fn checkpoint(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(15 + self.pending.len() + 32 * self.stack.len());
        out.extend(MAGIC);
        out.extend(self.chunks.to_be_bytes());
        out.extend((self.pending.len() as u16).to_be_bytes());
        out.push(self.stack.len() as u8);
        out.extend(&self.pending);
        for cv in &self.stack {
            out.extend(cv)
        }
        out
    }
    pub fn from_checkpoint(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 15 || &bytes[..4] != MAGIC {
            return Err(SyncError::Invalid("invalid hash checkpoint version".into()));
        }
        let chunks = u64::from_be_bytes(
            bytes[4..12]
                .try_into()
                .map_err(|_| SyncError::Invalid("invalid hash counter".into()))?,
        );
        let pending_len = u16::from_be_bytes(
            bytes[12..14]
                .try_into()
                .map_err(|_| SyncError::Invalid("invalid hash buffer".into()))?,
        ) as usize;
        let count = bytes[14] as usize;
        let expected = 15usize
            .checked_add(pending_len)
            .and_then(|n| n.checked_add(count.checked_mul(32)?))
            .ok_or(SyncError::LogicalLimit)?;
        if pending_len > CHUNK
            || count != chunks.count_ones() as usize
            || expected != bytes.len()
            || (chunks > 0 && pending_len == 0)
            || chunks
                .checked_mul(CHUNK as u64)
                .and_then(|n| n.checked_add(pending_len as u64))
                .is_none_or(|n| n > MAX_NUMBER)
        {
            return Err(SyncError::Invalid("invalid hash checkpoint".into()));
        }
        let pending = bytes[15..15 + pending_len].to_vec();
        let mut stack = Vec::with_capacity(count);
        for block in bytes[15 + pending_len..].as_chunks::<32>().0 {
            stack.push(*block);
        }
        Ok(Self {
            chunks,
            pending,
            stack,
        })
    }
}
