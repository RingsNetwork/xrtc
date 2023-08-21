use bytes::Bytes;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xrtc_core::message::XrtcMessage;
use xrtc_core::transport::SharedConnection;

use crate::error::TunnelDefeat;
use crate::message::ProxyMessage;

pub type TunnelId = Uuid;

pub(crate) struct Tunnel {
    tid: TunnelId,
    remote_stream_tx: Option<mpsc::Sender<Bytes>>,
    listener_cancel_token: Option<CancellationToken>,
    listener: Option<tokio::task::JoinHandle<()>>,
}

pub(crate) struct TunnelListener<C>
where C: SharedConnection
{
    tid: TunnelId,
    local_stream: TcpStream,
    remote_stream_tx: mpsc::Sender<Bytes>,
    remote_stream_rx: mpsc::Receiver<Bytes>,
    connection: C,
    cancel_token: CancellationToken,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Some(cancel_token) = self.listener_cancel_token.take() {
            cancel_token.cancel();
        }

        // Wait a time for listener to exit. Then abort it.
        if let Some(listener) = self.listener.take() {
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                listener.abort();
            });
        }
    }
}

impl Tunnel {
    pub fn new(tid: TunnelId) -> Self {
        Self {
            tid,
            remote_stream_tx: None,
            listener: None,
            listener_cancel_token: None,
        }
    }

    pub async fn send(&self, bytes: Bytes) {
        if let Some(ref tx) = self.remote_stream_tx {
            let _ = tx.send(bytes).await;
        } else {
            tracing::error!("Tunnel {} remote stream tx is none", self.tid);
        }
    }

    pub async fn listen<C>(&mut self, local_stream: TcpStream, connection: C)
    where C: SharedConnection {
        if self.listener.is_some() {
            return;
        }

        let mut listener = TunnelListener::new(self.tid, local_stream, connection).await;
        let listener_cancel_token = listener.cancel_token();
        let remote_stream_tx = listener.remote_stream_tx.clone();
        let listener_handler = tokio::spawn(Box::pin(async move { listener.listen().await }));

        self.remote_stream_tx = Some(remote_stream_tx);
        self.listener = Some(listener_handler);
        self.listener_cancel_token = Some(listener_cancel_token);
    }
}

impl<C> TunnelListener<C>
where C: SharedConnection
{
    async fn new(tid: TunnelId, local_stream: TcpStream, connection: C) -> Self {
        let (remote_stream_tx, remote_stream_rx) = mpsc::channel(1024);
        Self {
            tid,
            local_stream,
            remote_stream_tx,
            remote_stream_rx,
            connection,
            cancel_token: CancellationToken::new(),
        }
    }

    fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    async fn listen(&mut self) {
        let (mut local_read, mut local_write) = self.local_stream.split();

        let listen_local = async {
            loop {
                if self.cancel_token.is_cancelled() {
                    break TunnelDefeat::ConnectionClosed;
                }

                let mut buf = [0u8; 30000];
                match local_read.read(&mut buf).await {
                    Err(e) => {
                        break e.kind().into();
                    }
                    Ok(n) if n == 0 => {
                        break TunnelDefeat::ConnectionClosed;
                    }
                    Ok(n) => {
                        let body = Bytes::copy_from_slice(&buf[..n]);
                        let message = ProxyMessage::TcpPackage {
                            tid: self.tid,
                            body,
                        };

                        let Ok(xrtc_message) = message.try_into() else {
                            tracing::error!("Serialize ProxyMessage::TcpPackage failed");
                            break TunnelDefeat::SerializationFailed;
                        };

                        if let Err(e) = self.connection.send_message(xrtc_message).await {
                            tracing::error!("Send TcpPackage message failed: {e:?}");
                            break TunnelDefeat::WebrtcDatachannelSendFailed;
                        }
                    }
                }
            }
        };

        let listen_remote = async {
            loop {
                if self.cancel_token.is_cancelled() {
                    break TunnelDefeat::ConnectionClosed;
                }

                if let Some(body) = self.remote_stream_rx.recv().await {
                    if let Err(e) = local_write.write_all(&body).await {
                        tracing::error!("Write to local stream failed: {e:?}");
                        break e.kind().into();
                    }
                }
            }
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
