pub mod connection;

use std::sync::Arc;

use dashmap::DashMap;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;

use crate::callback::InnerCallback;
use crate::callback::XrtcMessage;
use crate::error::Error;
use crate::error::Result;
use crate::rtc::connection::ConnectionId;
use crate::rtc::connection::XrtcConnection;

pub type SharedTransport = Arc<Transport>;

pub struct Transport {
    webrtc_api: webrtc::api::API,
    webrtc_config: RTCConfiguration,
    connections: DashMap<ConnectionId, Arc<XrtcConnection>>,
}

impl Transport {
    fn new(ice_servers: Vec<String>) -> Self {
        let webrtc_api = webrtc::api::APIBuilder::new().build();
        let webrtc_config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: ice_servers,
                ..Default::default()
            }],
            ..Default::default()
        };

        Self {
            webrtc_api,
            webrtc_config,
            connections: DashMap::new(),
        }
    }

    pub fn new_shared(ice_servers: Vec<String>) -> Arc<Self> {
        Arc::new(Self::new(ice_servers))
    }

    pub fn get_connection(&self, cid: &ConnectionId) -> Option<Arc<XrtcConnection>> {
        self.connections.get(cid).map(|c| c.value().clone())
    }

    fn register_connection(&self, cid: ConnectionId, conn: Arc<XrtcConnection>) {
        self.connections.insert(cid, conn);
    }

    pub async fn send_message(&self, cid: &ConnectionId, msg: XrtcMessage) -> Result<()> {
        let Some(conn) = self.get_connection(cid) else {
            return Err(Error::ConnectionNotFound(cid.clone()));
        };
        conn.send_message(msg).await
    }

    pub(crate) async fn new_connection(
        self: Arc<Self>,
        cid: ConnectionId,
        callback: Arc<InnerCallback>,
    ) -> Result<()> {
        let webrtc_conn = self
            .webrtc_api
            .new_peer_connection(self.webrtc_config.clone())
            .await?;

        let conn_id = cid.clone();
        let callback = callback.clone();

        webrtc_conn.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
            let d_label = d.label();
            let d_id = d.id();
            tracing::debug!("New DataChannel {d_label} {d_id}");

            let conn_id = conn_id.clone();
            let callback = callback.clone();

            Box::pin(async move {
                // Register text message handling
                d.on_message(Box::new(move |msg: DataChannelMessage| {
                    tracing::debug!("Received DataChannelMessage from {conn_id}: {msg:?}");

                    let conn_id = conn_id.clone();
                    let callback = callback.clone();

                    Box::pin(async move {
                        callback.on_message(conn_id, &msg).await;
                    })
                }));
            })
        }));

        let xrtc_conn = XrtcConnection::new(webrtc_conn).await?;
        self.register_connection(cid, Arc::new(xrtc_conn));

        Ok(())
    }
}
