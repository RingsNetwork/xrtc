use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde::Serialize;
use webrtc::data_channel::data_channel_message::DataChannelMessage;

use crate::rtc::connection::ConnectionId;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum XrtcMessage {
    Custom(Vec<u8>),
}

#[async_trait]
pub trait Callback {
    async fn on_message(&self, cid: ConnectionId, msg: &[u8]);
    fn boxed(self) -> BoxedCallback
    where Self: Sized + Send + Sync + 'static {
        Box::new(self)
    }
}

pub type BoxedCallback = Box<dyn Callback + Send + Sync>;

pub(crate) struct InnerCallback {
    callback: BoxedCallback,
}

impl InnerCallback {
    pub fn new_shared(callback: BoxedCallback) -> Arc<Self> {
        Arc::new(Self { callback })
    }

    pub async fn on_message(&self, cid: ConnectionId, msg: &DataChannelMessage) {
        match bincode::deserialize::<XrtcMessage>(&msg.data) {
            Ok(m) => self.handle_message(cid, &m).await,
            Err(e) => {
                tracing::error!("Deserialize DataChannelMessage failed: {:?}", e);
            }
        };
    }

    async fn handle_message(&self, cid: ConnectionId, msg: &XrtcMessage) {
        match msg {
            XrtcMessage::Custom(bytes) => {
                self.callback.on_message(cid.clone(), bytes).await;
            }
        }
    }
}
