use crate::callback::BoxedCallback;
use crate::rtc::SharedTransport;

pub trait XrtcBackend {
    fn bind(&mut self, transport: SharedTransport);
    fn transport(&self) -> SharedTransport;
    fn callback(&self) -> BoxedCallback;
}
