use std::fmt;

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerId([u8; 32]);

impl PeerId {
    pub fn into_bytes(self) -> [u8; 32] {
        self.0
    }

    pub fn to_base58(self) -> String {
        bs58::encode(self.0).into_string()
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        PeerId(bytes)
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PeerId").field(&self.to_base58()).finish()
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_base58().fmt(f)
    }
}
