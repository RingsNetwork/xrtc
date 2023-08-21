mod callback;
pub mod connection;
pub mod error;

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use xrtc_core::callback::BoxedCallback;
use xrtc_core::message::XrtcMessage;
use xrtc_core::transport::SharedConnection;
use xrtc_core::transport::SharedTransport;

use crate::callback::InnerCallback;
use crate::connection::Connection;
use crate::error::Error;
use crate::error::Result;

#[derive(Clone)]
pub struct Transport {
    inner: Arc<TransportInner>,
}

struct TransportInner {
    webrtc_api: webrtc::api::API,
    webrtc_config: RTCConfiguration,
    connections: DashMap<String, Connection>,
}

impl Transport {
    pub fn new(ice_servers: Vec<String>) -> Self {
        let webrtc_api = webrtc::api::APIBuilder::new().build();
        let webrtc_config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: ice_servers,
                ..Default::default()
            }],
            ..Default::default()
        };

        Self {
            inner: Arc::new(TransportInner {
                webrtc_api,
                webrtc_config,
                connections: DashMap::new(),
            }),
        }
    }
}

#[async_trait]
impl SharedTransport for Transport {
    type Connection = Connection;
    type Error = Error;

    async fn new_connection<CE>(&self, cid: &str, callback: Arc<BoxedCallback<CE>>) -> Result<()>
    where CE: std::error::Error + Send + Sync + 'static {
        let webrtc_conn = self
            .inner
            .webrtc_api
            .new_peer_connection(self.inner.webrtc_config.clone())
            .await?;

        let conn_id = cid.to_string();
        let inner_cb = Arc::new(InnerCallback::new(callback));

        webrtc_conn.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
            let d_label = d.label();
            let d_id = d.id();
            tracing::debug!("New DataChannel {d_label} {d_id}");

            let conn_id = conn_id.clone();
            let inner_cb = inner_cb.clone();

            Box::pin(async move {
                d.on_message(Box::new(move |msg: DataChannelMessage| {
                    tracing::debug!("Received DataChannelMessage from {conn_id}: {msg:?}");

                    let conn_id = conn_id.clone();
                    let inner_cb = inner_cb.clone();

                    Box::pin(async move {
                        inner_cb.on_message(&conn_id, &msg.data).await;
                    })
                }));
            })
        }));

        let conn = Connection::new(webrtc_conn).await?;
        self.inner.register_connection(cid, conn);

        Ok(())
    }

    async fn send_message(&self, cid: &str, msg: XrtcMessage) -> Result<()> {
        let Some(conn) = self.inner.get_connection(cid) else {
            return Err(Error::ConnectionNotFound(cid.to_string()));
        };
        conn.send_message(msg).await
    }

    fn get_connection(&self, cid: &str) -> Option<Connection> {
        self.inner.get_connection(cid)
    }
}

impl TransportInner {
    fn get_connection(&self, cid: &str) -> Option<Connection> {
        self.connections.get(cid).map(|c| c.value().clone())
    }
    fn register_connection(&self, cid: &str, conn: Connection) {
        self.connections.insert(cid.to_string(), conn);
    }
}
