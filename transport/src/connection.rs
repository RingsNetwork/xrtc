use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use webrtc::data_channel::RTCDataChannel;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use xrtc_core::message::XrtcMessage;
use xrtc_core::transport::SharedConnection;

use crate::error::Error;
use crate::error::Result;

#[derive(Clone)]
pub struct Connection {
    inner: Arc<ConnectionInner>,
}

struct ConnectionInner {
    webrtc_conn: RTCPeerConnection,
    webrtc_data_channel: Arc<RTCDataChannel>,
}

impl Connection {
    pub async fn new(webrtc_conn: RTCPeerConnection) -> Result<Self> {
        let webrtc_data_channel = webrtc_conn.create_data_channel("xrtc", None).await?;
        Ok(Self {
            inner: Arc::new(ConnectionInner {
                webrtc_conn,
                webrtc_data_channel,
            }),
        })
    }
}

#[async_trait]
impl SharedConnection for Connection {
    type Sdp = RTCSessionDescription;
    type Error = Error;

    async fn send_message(&self, msg: XrtcMessage) -> Result<()> {
        let data = bincode::serialize(&msg).map(Bytes::from)?;
        self.inner.webrtc_data_channel.send(&data).await?;
        Ok(())
    }

    async fn webrtc_create_offer(&self) -> Result<Self::Sdp> {
        self.inner.webrtc_create_offer().await
    }

    async fn webrtc_answer_offer(&self, offer: Self::Sdp) -> Result<Self::Sdp> {
        self.inner.webrtc_answer_offer(offer).await
    }

    async fn webrtc_accept_answer(&self, answer: Self::Sdp) -> Result<()> {
        self.inner.webrtc_accept_answer(answer).await
    }
}

impl ConnectionInner {
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

    async fn webrtc_create_offer(&self) -> Result<RTCSessionDescription> {
        let setting_offer = self.webrtc_conn.create_offer(None).await?;
        self.webrtc_conn
            .set_local_description(setting_offer.clone())
            .await?;

        self.webrtc_gather().await
    }

    async fn webrtc_answer_offer(
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

    async fn webrtc_accept_answer(&self, answer: RTCSessionDescription) -> Result<()> {
        tracing::debug!("webrtc_accept_answer, answer: {answer:?}");

        self.webrtc_conn
            .set_remote_description(answer)
            .await
            .map_err(|e| e.into())
    }
}
