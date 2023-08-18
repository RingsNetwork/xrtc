pub mod backend;
pub mod callback;
pub mod error;
pub mod logging;
pub mod protocols;
pub mod rtc;
pub mod service;

use std::sync::Arc;

use crate::backend::XrtcBackend;
use crate::callback::InnerCallback;
use crate::error::Result;
use crate::rtc::connection::ConnectionId;
use crate::rtc::SharedTransport;
use crate::rtc::Transport;

pub struct XrtcServer<TBackend>
where TBackend: XrtcBackend
{
    transport: SharedTransport,
    callback: Arc<InnerCallback>,
    backend: TBackend,
}

impl<TBackend> XrtcServer<TBackend>
where TBackend: XrtcBackend
{
    pub fn new(ice_servers: Vec<String>, mut backend: TBackend) -> Self {
        let transport = Transport::new_shared(ice_servers);
        backend.bind(transport.clone());

        let cb = backend.callback();
        let callback = InnerCallback::new_shared(cb);

        Self {
            transport,
            callback,
            backend,
        }
    }

    pub fn backend(&self) -> &TBackend {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut TBackend {
        &mut self.backend
    }

    pub async fn new_connection(&self, cid: ConnectionId) -> Result<()> {
        self.transport
            .clone()
            .new_connection(cid, self.callback.clone())
            .await
    }
}
