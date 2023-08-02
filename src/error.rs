pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("WebRTC error: {0}")]
    Webrtc(#[from] webrtc::error::Error),

    #[error("Bincode error: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("WebRTC local SDP generation error")]
    WebrtcLocalSdpGenerationError,

    #[error("Connection not found, should handshake first")]
    ConnectionNotFound,
}
