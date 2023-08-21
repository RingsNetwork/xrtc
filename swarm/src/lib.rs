use std::sync::Arc;

use xrtc_core::callback::BoxedCallback;
use xrtc_core::transport::SharedTransport;

pub struct Swarm<T, B>
where
    T: SharedTransport,
    B: Backend,
{
    transport: T,
    backend: B,
    callback: Arc<BoxedCallback<B::Error>>,
}

pub trait Backend {
    type Error: std::error::Error + Send + Sync + 'static;
    fn callback(&self) -> BoxedCallback<Self::Error>;
}

impl<T, B> Swarm<T, B>
where
    T: SharedTransport,
    B: Backend,
{
    pub fn new(transport: T, backend: B) -> Self {
        let callback = Arc::new(backend.callback());
        Self {
            transport,
            backend,
            callback,
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub async fn new_connection(&self, cid: &str) -> Result<(), T::Error> {
        self.transport
            .new_connection(cid, self.callback.clone())
            .await
    }

    pub fn get_connection(&self, cid: &str) -> Option<T::Connection> {
        self.transport.get_connection(cid)
    }
}
