use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::callback::BoxedCallback;
use crate::message::XrtcMessage;

#[async_trait]
pub trait SharedTransport: Clone + Send + Sync + 'static {
    type Connection: SharedConnection<Error = Self::Error>;
    type Error: std::error::Error + Send + Sync + 'static;

    async fn new_connection<CE>(
        &self,
        cid: &str,
        callback: Arc<BoxedCallback<CE>>,
    ) -> Result<(), Self::Error>
    where
        CE: std::error::Error + Send + Sync + 'static;

    async fn send_message(&self, cid: &str, msg: XrtcMessage) -> Result<(), Self::Error>;

    fn get_connection(&self, cid: &str) -> Option<Self::Connection>;
}

#[async_trait]
pub trait SharedConnection: Clone + Send + Sync + 'static {
    type Sdp: Serialize + DeserializeOwned + Send + Sync;
    type Error: std::fmt::Debug;

    async fn send_message(&self, msg: XrtcMessage) -> Result<(), Self::Error>;

    async fn webrtc_create_offer(&self) -> Result<Self::Sdp, Self::Error>;
    async fn webrtc_answer_offer(&self, offer: Self::Sdp) -> Result<Self::Sdp, Self::Error>;
    async fn webrtc_accept_answer(&self, answer: Self::Sdp) -> Result<(), Self::Error>;
}
