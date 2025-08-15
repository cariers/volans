mod behavior;
mod handler;
mod shared;

pub use behavior::{Behavior, Controller};
pub use handler::Handler;

use volans_swarm::StreamProtocol;

#[derive(Debug)]
#[non_exhaustive]
pub enum StreamError {
    Unsupported(StreamProtocol),
    Io(std::io::Error),
}
