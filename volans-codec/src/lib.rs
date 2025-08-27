mod json;
mod protobuf;

pub use json::JsonUviCodec;
pub use protobuf::ProtobufUviCodec;

pub use asynchronous_codec::*;
pub use prost;

pub struct CombinedCodec<TEncoder, TDecoder>
where
    TEncoder: Encoder,
    TDecoder: Decoder,
{
    encoder: TEncoder,
    decoder: TDecoder,
}

impl<TEncoder, TDecoder> Encoder for CombinedCodec<TEncoder, TDecoder>
where
    TEncoder: Encoder,
    TDecoder: Decoder,
{
    type Item<'a> = TEncoder::Item<'a>;
    type Error = TEncoder::Error;

    fn encode(&mut self, item: Self::Item<'_>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        self.encoder.encode(item, dst)
    }
}

impl<TEncoder, TDecoder> Decoder for CombinedCodec<TEncoder, TDecoder>
where
    TEncoder: Encoder,
    TDecoder: Decoder,
{
    type Item = TDecoder::Item;
    type Error = TDecoder::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.decoder.decode(src)
    }
}

impl<TEncoder, TDecoder> Clone for CombinedCodec<TEncoder, TDecoder>
where
    TEncoder: Encoder + Clone,
    TDecoder: Decoder + Clone,
{
    fn clone(&self) -> Self {
        CombinedCodec {
            encoder: self.encoder.clone(),
            decoder: self.decoder.clone(),
        }
    }
}

impl<TEncoder, TDecoder> Default for CombinedCodec<TEncoder, TDecoder>
where
    TEncoder: Encoder + Default,
    TDecoder: Decoder + Default,
{
    fn default() -> Self {
        CombinedCodec {
            encoder: TEncoder::default(),
            decoder: TDecoder::default(),
        }
    }
}

impl<TEncoder, TDecoder> CombinedCodec<TEncoder, TDecoder>
where
    TEncoder: Encoder,
    TDecoder: Decoder,
{
    pub fn new(encoder: TEncoder, decoder: TDecoder) -> Self {
        CombinedCodec { encoder, decoder }
    }

    pub fn encoder(&self) -> &TEncoder {
        &self.encoder
    }

    pub fn decoder(&self) -> &TDecoder {
        &self.decoder
    }

    pub fn encoder_mut(&mut self) -> &mut TEncoder {
        &mut self.encoder
    }

    pub fn decoder_mut(&mut self) -> &mut TDecoder {
        &mut self.decoder
    }
}
