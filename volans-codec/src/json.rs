use std::io;

use asynchronous_codec::{Bytes, BytesMut, Decoder, Encoder};
use serde::{Serialize, de::DeserializeOwned};
use unsigned_varint::codec::UviBytes;

pub struct JsonUviCodec<M> {
    uvi_codec: UviBytes,
    _marker: std::marker::PhantomData<M>,
}

impl<M> Default for JsonUviCodec<M> {
    fn default() -> Self {
        JsonUviCodec {
            uvi_codec: UviBytes::default(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<M> Clone for JsonUviCodec<M> {
    fn clone(&self) -> Self {
        let mut uvi = UviBytes::default();
        uvi.set_max_len(self.uvi_codec.max_len());
        JsonUviCodec {
            uvi_codec: uvi,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<M> JsonUviCodec<M> {
    pub fn set_max_len(&mut self, val: usize) {
        self.uvi_codec.set_max_len(val)
    }

    pub fn max_len(&self) -> usize {
        self.uvi_codec.max_len()
    }
}

impl<M> Encoder for JsonUviCodec<M>
where
    M: Serialize,
{
    type Item<'a> = M;
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item<'_>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let buffer = serde_json::to_vec(&item)?;
        self.uvi_codec.encode(Bytes::from(buffer), dst)
    }
}

impl<M> Decoder for JsonUviCodec<M>
where
    M: DeserializeOwned,
{
    type Item = M;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.uvi_codec.decode(src) {
            Ok(Some(bytes)) => {
                let item = serde_json::from_slice(bytes.as_ref())?;
                Ok(Some(item))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        }
    }
}
