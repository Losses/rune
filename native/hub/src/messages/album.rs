use rinf::SignalPiece;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, SignalPiece, Debug)]
pub struct Album {
    pub id: i32,
    pub name: String,
}
