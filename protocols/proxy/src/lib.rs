pub mod backend;
pub mod error;
pub mod message;
pub mod tunnel;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::net::TcpStream;
use tokio::time::timeout;
use xrtc_core::transport::SharedConnection;
use xrtc_core::transport::SharedTransport;

use crate::error::Error;
use crate::error::TunnelDefeat;
use crate::message::ProxyMessage;
use crate::tunnel::Tunnel;
use crate::tunnel::TunnelId;

#[derive(Clone)]
pub struct Proxy<T>
where T: SharedTransport
{
    transport: T,
    tunnels: Arc<DashMap<TunnelId, Tunnel>>,
}

impl<T> Proxy<T>
where T: SharedTransport
{
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            tunnels: Arc::new(DashMap::new()),
        }
    }

    pub async fn dial(
        &self,
        cid: &str,
        remote_addr: SocketAddr,
        local_stream: TcpStream,
    ) -> Result<(), Error> {
        let conn = self
            .transport
            .get_connection(cid)
            .ok_or(Error::ConnectionNotFound(cid.to_string()))?;

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
        .map_err(|e| Error::TransportError(Box::new(e)))
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
