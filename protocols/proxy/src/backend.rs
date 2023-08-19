use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::TcpStream;
use tokio::time::timeout;
use xrtc_core::callback::BoxedCallback;
use xrtc_core::callback::Callback;
use xrtc_core::message::XrtcMessage;
use xrtc_core::transport::SharedTransport;
use xrtc_swarm::Backend;

use crate::error::Error;
use crate::error::TunnelDefeat;
use crate::message::ProxyMessage;
use crate::tunnel::Tunnel;
use crate::Proxy;

#[async_trait]
impl<T> Callback for Proxy<T>
where T: SharedTransport
{
    type Error = Error;

    async fn on_message(&self, cid: &str, msg: &[u8]) -> Result<(), Error> {
        let msg = bincode::deserialize::<ProxyMessage>(msg)?;

        match self.handle_message(cid, &msg).await {
            Ok(_) => Ok(()),
            Err(e) => {
                let tid = msg.tid();
                let close_msg = ProxyMessage::TcpClose { tid, reason: e };
                let xrtc_close_msg = XrtcMessage::try_from(close_msg)?;

                self.transport
                    .send_message(cid, xrtc_close_msg)
                    .await
                    .map_err(|e| Error::TransportError(Box::new(e)))?;

                Err(e.into())
            }
        }
    }
}

#[async_trait]
impl<T> Backend for Proxy<T>
where T: SharedTransport
{
    type Error = Error;

    fn callback(&self) -> BoxedCallback<Error> {
        self.clone().boxed()
    }
}

impl<T> Proxy<T>
where T: SharedTransport
{
    pub async fn handle_message(&self, cid: &str, msg: &ProxyMessage) -> Result<(), TunnelDefeat> {
        match msg {
            ProxyMessage::TcpDial { tid, addr } => {
                let conn = self
                    .transport
                    .get_connection(cid)
                    .ok_or(TunnelDefeat::WebrtcConnectionNotFound)?;

                let local_stream = tcp_connect_with_timeout(addr, 10).await?;

                let mut tunnel = Tunnel::new(*tid);
                tunnel.listen(local_stream, conn.clone()).await;

                self.tunnels.insert(*tid, tunnel);
                Ok(())
            }

            ProxyMessage::TcpClose { tid, reason } => {
                tracing::warn!("Tunnel {tid} close: {reason:?}");
                self.tunnels.remove(tid);
                Ok(())
            }

            ProxyMessage::TcpPackage { tid, body } => {
                let tunnel = self.tunnels.get(tid).ok_or(TunnelDefeat::TunnelNotFound)?;

                // TODO: prevent body clone
                let bytes = body.clone();

                // TODO: prevent send huge bytes
                tunnel.send(bytes).await;

                Ok(())
            }
        }
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
