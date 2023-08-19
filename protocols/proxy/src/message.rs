use std::net::SocketAddr;

use bytes::Bytes;
use serde::Deserialize;
use serde::Serialize;
use xrtc_core::message::XrtcMessage;

use crate::error::Error;
use crate::error::TunnelDefeat;
use crate::tunnel::TunnelId;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum ProxyMessage {
    TcpDial { tid: TunnelId, addr: SocketAddr },
    TcpClose { tid: TunnelId, reason: TunnelDefeat },
    TcpPackage { tid: TunnelId, body: Bytes },
}

impl TryFrom<ProxyMessage> for XrtcMessage {
    type Error = Error;

    fn try_from(msg: ProxyMessage) -> Result<Self, Self::Error> {
        let bytes = bincode::serialize(&msg).map_err(Error::Bincode)?;
        Ok(XrtcMessage::Custom(bytes))
    }
}

impl ProxyMessage {
    pub fn tid(&self) -> TunnelId {
        match self {
            ProxyMessage::TcpDial { tid, .. } => *tid,
            ProxyMessage::TcpClose { tid, .. } => *tid,
            ProxyMessage::TcpPackage { tid, .. } => *tid,
        }
    }
}
