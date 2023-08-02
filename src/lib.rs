pub mod error;
pub mod logging;
pub mod proxy;
pub mod service;

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use dashmap::DashMap;
use error::Error;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use uuid::Uuid;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

use crate::error::Result;

type ConnectionId = String;
type TransactionId = Uuid;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum XrtcMessage {
    TcpDial { tx: TransactionId, addr: SocketAddr },
    TcpClose { tx: TransactionId },
    TcpInboundPackage { tx: TransactionId, body: Bytes },
    TcpOutboundPackage { tx: TransactionId, body: Bytes },
}

pub struct XrtcServer {
    pub webrtc_api: webrtc::api::API,
    pub webrtc_config: RTCConfiguration,
    pub data_channel_message_tx: mpsc::Sender<(ConnectionId, XrtcMessage)>,
    pub data_channel_message_rx: Mutex<mpsc::Receiver<(ConnectionId, XrtcMessage)>>,
    pub connections: DashMap<ConnectionId, Arc<XrtcConnection>>,
    pub transactions: DashMap<TransactionId, Arc<XrtcConnection>>,
}

pub struct XrtcConnection {
    webrtc_conn: RTCPeerConnection,
    webrtc_data_channel: Arc<RTCDataChannel>,
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
            transactions: DashMap::new(),
        }
    }

    pub async fn run(&self) {
        let mut data_channel_message_rx = self.data_channel_message_rx.lock().await;

        loop {
            if let Some((cid, msg)) = data_channel_message_rx.recv().await {
                tracing::debug!("Received XrtcMessage from {cid}: {msg:?}");
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
        };

        self.connections.insert(cid, Arc::new(xrtc_conn));

        Ok(())
    }

    pub async fn send_message(&self, cid: ConnectionId, msg: XrtcMessage) -> Result<()> {
        let conn = self
            .connections
            .get(&cid)
            .ok_or(Error::ConnectionNotFound)?;

        let data = bincode::serialize(&msg).map(Bytes::from)?;
        conn.webrtc_data_channel.send(&data).await?;

        Ok(())
    }

    // Proxy communication
    pub async fn dial(&self, cid: ConnectionId, addr: SocketAddr) -> Result<()> {
        let message = XrtcMessage::TcpDial {
            tx: Uuid::new_v4(),
            addr,
        };
        self.send_message(cid, message).await
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
}
