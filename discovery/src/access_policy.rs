use std::error::Error;

use tokio::sync::watch;

/// A peer whose key and fingerprint were verified by the transport.
#[derive(Clone, Debug)]
pub struct PeerIdentity {
    pub fingerprint: String,
    pub public_key: String,
}

/// Application authorization, independent of pairing trust.
pub trait AccessPolicy: Send + Sync + 'static {
    type Action: Send + Sync;
    type Denial: Error + Send + Sync;

    /// Decide from an in-memory snapshot, without asynchronous work or I/O.
    fn check(&self, peer: &PeerIdentity, action: &Self::Action) -> Result<(), Self::Denial>;
    fn changes(&self) -> watch::Receiver<u64>;
}
