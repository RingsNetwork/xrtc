pub mod error;
pub mod logging;
pub mod proxy;
pub mod service;

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use dashmap::DashMap;
use error::Error;
use proxy::tcp_connect_with_timeout;
use serde::Deserialize;
use serde::Serialize;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

use crate::error::Result;
use crate::proxy::Tunnel;
use crate::proxy::TunnelDefeat;
use crate::proxy::TunnelId;

pub type ConnectionId = String;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum XrtcMessage {
    TcpDial { tid: TunnelId, addr: SocketAddr },
    TcpClose { tid: TunnelId, reason: TunnelDefeat },
    TcpPackage { tid: TunnelId, body: Bytes },
}

pub struct XrtcServer {
    pub webrtc_api: webrtc::api::API,
    pub webrtc_config: RTCConfiguration,
    pub data_channel_message_tx: mpsc::Sender<(ConnectionId, XrtcMessage)>,
    pub data_channel_message_rx: Mutex<mpsc::Receiver<(ConnectionId, XrtcMessage)>>,
    pub connections: DashMap<ConnectionId, Arc<XrtcConnection>>,
}

pub struct XrtcConnection {
    webrtc_conn: RTCPeerConnection,
    webrtc_data_channel: Arc<RTCDataChannel>,
    tunnels: DashMap<TunnelId, Tunnel>,
}

impl XrtcServer {
    pub fn new(ice_servers: Vec<String>) -> Self {
        let webrtc_api = webrtc::api::APIBuilder::new().build();
        let webrtc_config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: ice_servers,
                ..Default::default()
            }],
            ..Default::default()
        };

        let (data_channel_message_tx, data_channel_message_rx_inner) = mpsc::channel(1024);
        let data_channel_message_rx = Mutex::new(data_channel_message_rx_inner);

        Self {
            webrtc_api,
            webrtc_config,
            data_channel_message_tx,
            data_channel_message_rx,
            connections: DashMap::new(),
        }
    }

    pub async fn run(&self) {
        tokio::join!(
            self.listen_webrtc_connections(),
            self.listen_tcp_connections()
        );
    }

    pub async fn listen_webrtc_connections(&self) {
        let mut data_channel_message_rx = self.data_channel_message_rx.lock().await;

        loop {
            if let Some((cid, msg)) = data_channel_message_rx.recv().await {
                tracing::info!("Received message from {cid}: {msg:?}");
                if let Err(e) = self.handle_message(cid, msg).await {
                    tracing::error!("Error handling message: {e}");
                }
            }
        }
    }

    pub async fn listen_tcp_connections(&self) {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    pub async fn handle_message(&self, cid: ConnectionId, msg: XrtcMessage) -> Result<()> {
        match msg {
            XrtcMessage::TcpDial { tid, addr } => {
                let conn = self
                    .connections
                    .get(&cid)
                    .ok_or(Error::ConnectionNotFound)?;

                match tcp_connect_with_timeout(addr, 10).await {
                    Err(e) => {
                        if let Err(e) = conn
                            .send_message(XrtcMessage::TcpClose { tid, reason: e })
                            .await
                        {
                            tracing::error!("Tunnel {tid} send tcp close message failed: {e}")
                        };

                        Err(Error::TunnelError(e))
                    }

                    Ok(local_stream) => {
                        let mut tunnel = Tunnel::new(tid);
                        tunnel.listen(local_stream, conn.clone()).await;
                        conn.tunnels.insert(tid, tunnel);
                        Ok(())
                    }
                }
            }

            XrtcMessage::TcpClose {
                tid,
                reason: _reason,
            } => {
                let conn = self
                    .connections
                    .get(&cid)
                    .ok_or(Error::ConnectionNotFound)?;

                conn.tunnels.remove(&tid);
                Ok(())
            }

            XrtcMessage::TcpPackage { tid, body } => {
                let conn = self
                    .connections
                    .get(&cid)
                    .ok_or(Error::ConnectionNotFound)?;

                conn.tunnels
                    .get(&tid)
                    .ok_or(Error::TunnelNotFound)?
                    .send(body)
                    .await;

                Ok(())
            }
        }
    }

    pub async fn new_connection(&self, cid: ConnectionId) -> Result<()> {
        let webrtc_conn = self
            .webrtc_api
            .new_peer_connection(self.webrtc_config.clone())
            .await?;

        let message_tx = self.data_channel_message_tx.clone();
        let conn_id = cid.clone();

        webrtc_conn.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
            let d_label = d.label();
            let d_id = d.id();
            tracing::debug!("New DataChannel {d_label} {d_id}");

            let message_tx = message_tx.clone();
            let conn_id = conn_id.clone();

            Box::pin(async move {
                // Register text message handling
                d.on_message(Box::new(move |msg: DataChannelMessage| {
                    tracing::debug!("Received DataChannelMessage from {conn_id}: {msg:?}");

                    let message_tx = message_tx.clone();
                    let conn_id = conn_id.clone();

                    Box::pin(async move {
                        match bincode::deserialize::<XrtcMessage>(&msg.data) {
                            Ok(m) => {
                                if let Err(e) = message_tx.send((conn_id, m)).await {
                                    tracing::error!("Send XrtcMessage via channel failed: {:?}", e);
                                }
                            }
                            Err(e) => {
                                tracing::error!("Deserialize DataChannelMessage failed: {:?}", e);
                            }
                        };
                    })
                }));
            })
        }));

        let webrtc_data_channel = webrtc_conn.create_data_channel("xrtc", None).await?;
        let xrtc_conn = XrtcConnection {
            webrtc_conn,
            webrtc_data_channel,
            tunnels: DashMap::new(),
        };

        self.connections.insert(cid, Arc::new(xrtc_conn));

        Ok(())
    }

    pub async fn dial(
        &self,
        cid: ConnectionId,
        remote_addr: SocketAddr,
        local_stream: TcpStream,
    ) -> Result<()> {
        let conn = self
            .connections
            .get(&cid)
            .ok_or(Error::ConnectionNotFound)?;

        let tid = uuid::Uuid::new_v4();
        let mut tunnel = Tunnel::new(tid);
        tunnel.listen(local_stream, conn.clone()).await;

        conn.tunnels.insert(tid, tunnel);
        conn.send_message(XrtcMessage::TcpDial {
            tid,
            addr: remote_addr,
        })
        .await
    }
}

impl XrtcConnection {
    async fn webrtc_gather(&self) -> Result<RTCSessionDescription> {
        self.webrtc_conn
            .gathering_complete_promise()
            .await
            .recv()
            .await;

        self.webrtc_conn
            .local_description()
            .await
            .ok_or(Error::WebrtcLocalSdpGenerationError)
    }

    pub async fn webrtc_create_offer(&self) -> Result<RTCSessionDescription> {
        let setting_offer = self.webrtc_conn.create_offer(None).await?;
        self.webrtc_conn
            .set_local_description(setting_offer.clone())
            .await?;

        self.webrtc_gather().await
    }

    pub async fn webrtc_answer_offer(
        &self,
        offer: RTCSessionDescription,
    ) -> Result<RTCSessionDescription> {
        tracing::debug!("webrtc_answer_offer, offer: {offer:?}");

        self.webrtc_conn.set_remote_description(offer).await?;

        let answer = self.webrtc_conn.create_answer(None).await?;
        self.webrtc_conn
            .set_local_description(answer.clone())
            .await?;

        self.webrtc_gather().await
    }

    pub async fn webrtc_accept_answer(&self, answer: RTCSessionDescription) -> Result<()> {
        tracing::debug!("webrtc_accept_answer, answer: {answer:?}");

        self.webrtc_conn
            .set_remote_description(answer)
            .await
            .map_err(|e| e.into())
    }

    async fn send_message(&self, msg: XrtcMessage) -> Result<()> {
        let data = bincode::serialize(&msg).map(Bytes::from)?;
        self.webrtc_data_channel.send(&data).await?;
        Ok(())
    }
}
