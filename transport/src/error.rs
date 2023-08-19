pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WebRTC error: {0}")]
    Webrtc(#[from] webrtc::error::Error),

    #[error("Bincode error: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("WebRTC local SDP generation error")]
    WebrtcLocalSdpGenerationError,

    #[error("Connection {0} not found, should handshake first")]
    ConnectionNotFound(String),
}
