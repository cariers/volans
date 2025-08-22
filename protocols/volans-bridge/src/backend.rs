pub mod handler;

pub mod behavior;

pub use behavior::Behavior;

use crate::transport;

pub fn new() -> (transport::Config, Behavior) {
    let (transport, transport_receiver) = transport::Config::new();
    let behavior = Behavior::new(transport_receiver);
    (transport, behavior)
}
