use std::io;

use asynchronous_codec::{BytesMut, Decoder, Encoder};
use unsigned_varint::codec::UviBytes;

pub struct ProtobufUviCodec<M> {
    uvi_codec: UviBytes,
    _marker: std::marker::PhantomData<M>,
}

impl<M> Default for ProtobufUviCodec<M> {
    fn default() -> Self {
        ProtobufUviCodec {
            uvi_codec: UviBytes::default(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<M> Clone for ProtobufUviCodec<M> {
    fn clone(&self) -> Self {
        let mut uvi = UviBytes::default();
        uvi.set_max_len(self.uvi_codec.max_len());
        ProtobufUviCodec {
            uvi_codec: uvi,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<M> ProtobufUviCodec<M> {
    pub fn set_max_len(&mut self, val: usize) {
        self.uvi_codec.set_max_len(val)
    }

    pub fn max_len(&self) -> usize {
        self.uvi_codec.max_len()
    }
}

impl<M> Encoder for ProtobufUviCodec<M>
where
    M: prost::Message,
{
    type Item<'a> = M;
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item<'_>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let len = item.encoded_len();
        let mut buffer = BytesMut::with_capacity(len);
        item.encode(&mut buffer)?;
        self.uvi_codec.encode(buffer.freeze(), dst)
    }
}

impl<M> Decoder for ProtobufUviCodec<M>
where
    M: prost::Message + Default,
{
    type Item = M;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.uvi_codec.decode(src) {
            Ok(Some(bytes)) => {
                let item = M::decode(bytes.as_ref())
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok(Some(item))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        }
    }
}
