use std::sync::Arc;

use bytes::Bytes;
use xrtc_core::callback::BoxedCallback;
use xrtc_core::message::XrtcMessage;

pub(crate) struct InnerCallback<CE> {
    callback: Arc<BoxedCallback<CE>>,
}

impl<CE> InnerCallback<CE>
where CE: std::error::Error + Send + Sync + 'static
{
    pub fn new(callback: Arc<BoxedCallback<CE>>) -> Self {
        Self { callback }
    }

    pub async fn on_message(&self, cid: &str, msg: &Bytes) {
        match bincode::deserialize(msg) {
            Ok(m) => self.handle_message(cid, &m).await,
            Err(e) => {
                tracing::error!("Deserialize DataChannelMessage failed: {:?}", e);
            }
        };
    }

    async fn handle_message(&self, cid: &str, msg: &XrtcMessage) {
        match msg {
            XrtcMessage::Custom(bytes) => {
                if let Err(e) = self.callback.on_message(cid, bytes).await {
                    tracing::error!("Callback on_message failed: {e:?}")
                }
            }
        }
    }
}
