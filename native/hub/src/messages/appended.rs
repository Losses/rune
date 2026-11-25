//! Reading messages that gained fields after protocol 2.0. Signals are positional, so a field
//! can only be added at the end of a message, and a peer of an older protocol ends the message
//! before it. A peer of an older protocol reads a newer message as far as its own fields go.

use serde::Deserialize;
use serde::de::{Error, Expected, SeqAccess};

/// Reads the next field of a message, which every peer sends.
pub(super) fn field<'de, A, T>(
    seq: &mut A,
    index: usize,
    expected: &dyn Expected,
) -> Result<T, A::Error>
where
    A: SeqAccess<'de>,
    T: Deserialize<'de>,
{
    seq.next_element()?
        .ok_or_else(|| Error::invalid_length(index, expected))
}

/// Reads a field appended after protocol 2.0, or its default when the message ends before it.
pub(super) fn appended_field<'de, A, T>(seq: &mut A) -> T
where
    A: SeqAccess<'de>,
    T: Deserialize<'de> + Default,
{
    seq.next_element().ok().flatten().unwrap_or_default()
}
