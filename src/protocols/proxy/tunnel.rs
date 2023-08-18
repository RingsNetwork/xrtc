use std::sync::Arc;

use bytes::Bytes;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::callback::XrtcMessage;
use crate::error::TunnelDefeat;
use crate::protocols::proxy::ProxyMessage;
use crate::rtc::connection::XrtcConnection;

pub type TunnelId = Uuid;

pub(crate) struct Tunnel {
    tid: TunnelId,
    remote_stream_tx: Option<mpsc::Sender<Bytes>>,
    listener: Option<tokio::task::JoinHandle<()>>,
}

pub(crate) struct TunnelListener {
    tid: TunnelId,
    local_stream: TcpStream,
    remote_stream_tx: mpsc::Sender<Bytes>,
    remote_stream_rx: mpsc::Receiver<Bytes>,
    connection: Arc<XrtcConnection>,
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
                        let message = ProxyMessage::TcpPackage {
                            tid: self.tid,
                            body,
                        };

                        let Ok(xrtc_message) = message.try_into() else {
                            tracing::error!("Serialize ProxyMessage::TcpPackage failed");
                            defeat = TunnelDefeat::SerializationFailed;
                            break;
                        };

                        if let Err(e) = self.connection.send_message(xrtc_message).await {
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
                tracing::warn!("Local stream closed: {defeat:?}");
                self.send_defeat(defeat).await;
            },
            defeat = listen_remote => {
                tracing::warn!("Remote stream closed: {defeat:?}");
                self.send_defeat(defeat).await;
            }
        }
    }

    async fn send_defeat(&self, defeat: TunnelDefeat) {
        let message = ProxyMessage::TcpClose {
            tid: self.tid,
            reason: defeat,
        };

        let xrtc_message: XrtcMessage = message.try_into().unwrap();

        if let Err(e) = self.connection.send_message(xrtc_message).await {
            tracing::error!("Send TcpClose message failed: {e:?}");
        }
    }
}