use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::ErrorKind as IOErrorKind;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use uuid::Uuid;

use crate::XrtcConnection;
use crate::XrtcMessage;

pub type TunnelId = Uuid;

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
pub enum TunnelDefeat {
    None = 0,
    WebrtcDatachannelSendFailed = 1,
    ConnectionTimeout = 2,
    ConnectionRefused = 3,
    ConnectionAborted = 4,
    ConnectionReset = 5,
    NotConnected = 6,
    ConnectionClosed = 7,
    Unknown = 255,
}

pub struct Tunnel {
    tid: TunnelId,
    remote_stream_tx: Option<mpsc::Sender<Bytes>>,
    listener: Option<tokio::task::JoinHandle<()>>,
}

pub struct TunnelListener {
    tid: TunnelId,
    local_stream: TcpStream,
    remote_stream_tx: mpsc::Sender<Bytes>,
    remote_stream_rx: mpsc::Receiver<Bytes>,
    connection: Arc<XrtcConnection>,
}

impl From<IOErrorKind> for TunnelDefeat {
    fn from(kind: IOErrorKind) -> TunnelDefeat {
        match kind {
            IOErrorKind::ConnectionRefused => TunnelDefeat::ConnectionRefused,
            IOErrorKind::ConnectionAborted => TunnelDefeat::ConnectionAborted,
            IOErrorKind::ConnectionReset => TunnelDefeat::ConnectionReset,
            IOErrorKind::NotConnected => TunnelDefeat::NotConnected,
            _ => TunnelDefeat::Unknown,
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Some(listener) = self.listener.take() {
            listener.abort();
        }
    }
}

impl Tunnel {
    pub fn new(tid: TunnelId) -> Self {
        Self {
            tid,
            remote_stream_tx: None,
            listener: None,
        }
    }

    pub async fn send(&self, bytes: Bytes) {
        if let Some(ref tx) = self.remote_stream_tx {
            let _ = tx.send(bytes).await;
        } else {
            tracing::error!("Tunnel {} remote stream tx is none", self.tid);
        }
    }

    pub async fn listen(&mut self, local_stream: TcpStream, connection: Arc<XrtcConnection>) {
        if self.listener.is_some() {
            return;
        }

        let mut listener = TunnelListener::new(self.tid, local_stream, connection.clone()).await;
        let remote_stream_tx = listener.remote_stream_tx.clone();
        let listener_handler = tokio::spawn(Box::pin(async move { listener.listen().await }));

        self.remote_stream_tx = Some(remote_stream_tx);
        self.listener = Some(listener_handler);
    }
}

impl TunnelListener {
    async fn new(tid: TunnelId, local_stream: TcpStream, connection: Arc<XrtcConnection>) -> Self {
        let (remote_stream_tx, remote_stream_rx) = mpsc::channel(1024);
        Self {
            tid,
            local_stream,
            remote_stream_tx,
            remote_stream_rx,
            connection,
        }
    }

    async fn listen(&mut self) {
        let (mut local_read, mut local_write) = self.local_stream.split();

        let listen_local = async {
            let mut defeat = TunnelDefeat::None;

            loop {
                let mut buf = [0u8; 30000];

                match local_read.read(&mut buf).await {
                    Err(e) => {
                        defeat = e.kind().into();
                        break;
                    }
                    Ok(n) if n == 0 => {
                        defeat = TunnelDefeat::ConnectionClosed;
                        break;
                    }
                    Ok(n) => {
                        let body = Bytes::copy_from_slice(&buf[..n]);
                        let message = XrtcMessage::TcpPackage {
                            tid: self.tid,
                            body,
                        };

                        if let Err(e) = self.connection.send_message(message).await {
                            tracing::error!("Send TcpPackage message failed: {e:?}");
                            defeat = TunnelDefeat::WebrtcDatachannelSendFailed;
                            break;
                        }
                    }
                }
            }

            defeat
        };

        let listen_remote = async {
            let mut defeat = TunnelDefeat::None;

            loop {
                if let Some(body) = self.remote_stream_rx.recv().await {
                    if let Err(e) = local_write.write_all(&body).await {
                        tracing::error!("Write to local stream failed: {e:?}");
                        defeat = e.kind().into();
                        break;
                    }
                }
            }

            defeat
        };

        tokio::select! {
            defeat = listen_local => {
                tracing::info!("Local stream closed: {defeat:?}");
                let message = XrtcMessage::TcpClose {
                    tid: self.tid,
                    reason: defeat,
                };
                if let Err(e) =  self.connection.send_message(message).await {
                    tracing::error!("Send TcpClose message failed: {e:?}");
                }
            },
            defeat = listen_remote => {
                tracing::info!("Remote stream closed: {defeat:?}");
                let message = XrtcMessage::TcpClose {
                    tid: self.tid,
                    reason: defeat,
                };
                let _ = self.connection.send_message(message).await;
            }
        }
    }
}

pub async fn tcp_connect_with_timeout(
    addr: SocketAddr,
    request_timeout_s: u64,
) -> Result<TcpStream, TunnelDefeat> {
    let fut = tcp_connect(addr);
    match timeout(Duration::from_secs(request_timeout_s), fut).await {
        Ok(result) => result,
        Err(_) => Err(TunnelDefeat::ConnectionTimeout),
    }
}

async fn tcp_connect(addr: SocketAddr) -> Result<TcpStream, TunnelDefeat> {
    match TcpStream::connect(addr).await {
        Ok(o) => Ok(o),
        Err(e) => Err(e.kind().into()),
    }
}
