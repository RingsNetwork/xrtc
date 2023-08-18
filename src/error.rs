use std::io::ErrorKind as IOErrorKind;

use serde::Deserialize;
use serde::Serialize;

use crate::rtc::connection::ConnectionId;

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
    ConnectionNotFound(ConnectionId),

    #[error("Tunnel error: {0:?}")]
    TunnelError(TunnelDefeat),
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
pub enum TunnelDefeat {
    None = 0,
    ConnectionTimeout = 1,
    ConnectionRefused = 2,
    ConnectionAborted = 3,
    ConnectionReset = 4,
    NotConnected = 5,
    ConnectionClosed = 6,
    WebrtcConnectionNotFound = 100,
    WebrtcDatachannelSendFailed = 101,
    TunnelNotFound = 200,
    SerializationFailed = 201,
    Unknown = 255,
}

impl From<IOErrorKind> for TunnelDefeat {
    fn from(kind: IOErrorKind) -> TunnelDefeat {
        match kind {
            IOErrorKind::ConnectionRefused => TunnelDefeat::ConnectionRefused,
            IOErrorKind::ConnectionAborted => TunnelDefeat::ConnectionAborted,
            IOErrorKind::ConnectionReset => TunnelDefeat::ConnectionReset,
            IOErrorKind::NotConnected => TunnelDefeat::NotConnected,
            _ => TunnelDefeat::Unknown,
        }
    }
}

impl From<TunnelDefeat> for Error {
    fn from(defeat: TunnelDefeat) -> Error {
        Error::TunnelError(defeat)
    }
}
