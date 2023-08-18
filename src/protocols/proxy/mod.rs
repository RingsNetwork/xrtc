mod backend;
pub mod tunnel;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use dashmap::DashMap;
use serde::Deserialize;
use serde::Serialize;
use tokio::net::TcpStream;
use tokio::time::timeout;
use uuid::Uuid;

use crate::backend::XrtcBackend;
use crate::callback::XrtcMessage;
use crate::error::Error;
use crate::error::TunnelDefeat;
use crate::protocols::proxy::tunnel::Tunnel;
use crate::rtc::connection::ConnectionId;
use crate::rtc::SharedTransport;

pub type TunnelId = Uuid;

#[derive(Clone, Default)]
pub struct XrtcProxy {
    transport: Option<SharedTransport>,
    tunnels: Arc<DashMap<TunnelId, Tunnel>>,
}

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
    fn tid(&self) -> TunnelId {
        match self {
            ProxyMessage::TcpDial { tid, .. } => *tid,
            ProxyMessage::TcpClose { tid, .. } => *tid,
            ProxyMessage::TcpPackage { tid, .. } => *tid,
        }
    }
}

impl XrtcProxy {
    pub fn new() -> Self {
        Self {
            transport: None,
            tunnels: Arc::new(DashMap::new()),
        }
    }

    pub async fn dial(
        &self,
        cid: ConnectionId,
        remote_addr: SocketAddr,
        local_stream: TcpStream,
    ) -> Result<(), Error> {
        let conn = self
            .transport()
            .get_connection(&cid)
            .ok_or(Error::ConnectionNotFound(cid.clone()))?;

        let tid = uuid::Uuid::new_v4();
        let mut tunnel = Tunnel::new(tid);
        tunnel.listen(local_stream, conn.clone()).await;

        self.tunnels.insert(tid, tunnel);
        conn.send_message(
            ProxyMessage::TcpDial {
                tid,
                addr: remote_addr,
            }
            .try_into()?,
        )
        .await
    }
}

pub async fn tcp_connect_with_timeout(
    addr: &SocketAddr,
    request_timeout_s: u64,
) -> Result<TcpStream, TunnelDefeat> {
    let fut = tcp_connect(addr);
    match timeout(Duration::from_secs(request_timeout_s), fut).await {
        Ok(result) => result,
        Err(_) => Err(TunnelDefeat::ConnectionTimeout),
    }
}

async fn tcp_connect(addr: &SocketAddr) -> Result<TcpStream, TunnelDefeat> {
    match TcpStream::connect(addr).await {
        Ok(o) => Ok(o),
        Err(e) => Err(e.kind().into()),
    }
}
