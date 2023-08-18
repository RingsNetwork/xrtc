use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::backend::XrtcBackend;
use crate::callback::BoxedCallback;
use crate::callback::Callback;
use crate::callback::XrtcMessage;
use crate::error::TunnelDefeat;
use crate::protocols::proxy::tunnel::Tunnel;
use crate::protocols::proxy::ProxyMessage;
use crate::protocols::proxy::XrtcProxy;
use crate::rtc::connection::ConnectionId;
use crate::rtc::SharedTransport;

#[async_trait]
impl Callback for XrtcProxy {
    async fn on_message(&self, cid: ConnectionId, msg: &[u8]) {
        match bincode::deserialize::<ProxyMessage>(msg) {
            Ok(m) => {
                let tid = m.tid();
                if let Err(e) = self.handle_message(cid.clone(), &m).await {
                    tracing::error!("{tid} handle ProxyMessage failed: {e:?}");

                    let close_msg = ProxyMessage::TcpClose { tid, reason: e };

                    match XrtcMessage::try_from(close_msg) {
                        Ok(m) => {
                            if let Err(e) = self.transport().send_message(&cid, m).await {
                                tracing::error!("Tunnel {tid} send tcp close message failed: {e}")
                            };
                        }
                        Err(e) => {
                            tracing::error!("Serialize ProxyMessage::TcpClose failed: {e:?}");
                        }
                    }
                }
            }
            Err(e) => tracing::error!("Deserialize ProxyMessage failed: {e:?}"),
        };
    }
}

#[async_trait]
impl XrtcBackend for XrtcProxy {
    fn bind(&mut self, transport: SharedTransport) {
        self.transport = Some(transport);
    }

    fn transport(&self) -> SharedTransport {
        self.transport.clone().expect("Should bind transport first")
    }

    fn callback(&self) -> BoxedCallback {
        self.clone().boxed()
    }
}

impl XrtcProxy {
    pub async fn handle_message(
        &self,
        cid: ConnectionId,
        msg: &ProxyMessage,
    ) -> Result<(), TunnelDefeat> {
        match msg {
            ProxyMessage::TcpDial { tid, addr } => {
                let conn = self
                    .transport()
                    .get_connection(&cid)
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
