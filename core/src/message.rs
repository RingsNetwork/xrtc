use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum XrtcMessage {
    Custom(Vec<u8>),
}
