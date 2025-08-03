use rinf::SignalPiece;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct Artist {
    pub id: i32,
    pub name: String,
}
