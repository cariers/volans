use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{AsyncRead, AsyncWrite, Sink, Stream, ready};
use pin_project::pin_project;

const MAX_LENGTH_SIZE: usize = 4;
const MAX_FRAME_SIZE: u32 = u32::MAX >> MAX_LENGTH_SIZE;
const DEFAULT_BUFFER_SIZE: usize = 128;

#[pin_project]
#[derive(Debug)]
pub(crate) struct LengthDelimited<R> {
    #[pin]
    inner: R,
    read_buffer: BytesMut,
    write_buffer: BytesMut,
    read_state: ReadState,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ReadState {
    ReadLength {
        buf: [u8; MAX_LENGTH_SIZE],
        pos: usize,
    },
    ReadData {
        len: u32,
        pos: usize,
    },
}

impl Default for ReadState {
    fn default() -> Self {
        ReadState::ReadLength {
            buf: [0; MAX_LENGTH_SIZE],
            pos: 0,
        }
    }
}

impl<R> LengthDelimited<R> {
    pub(crate) fn new(inner: R) -> LengthDelimited<R> {
        LengthDelimited {
            inner,
            read_state: ReadState::default(),
            read_buffer: BytesMut::with_capacity(DEFAULT_BUFFER_SIZE),
            write_buffer: BytesMut::with_capacity(DEFAULT_BUFFER_SIZE + MAX_LENGTH_SIZE as usize),
        }
    }

    pub(crate) fn into_inner(self) -> R {
        assert!(self.read_buffer.is_empty());
        assert!(self.write_buffer.is_empty());
        self.inner
    }

    pub(crate) fn into_reader(self) -> LengthDelimitedReader<R> {
        LengthDelimitedReader { inner: self }
    }

    /// 写入所有数据到底层I/O流
    fn poll_write_buffer(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>>
    where
        R: AsyncWrite,
    {
        let mut this = self.project();
        while !this.write_buffer.is_empty() {
            let len = ready!(this.inner.as_mut().poll_write(cx, this.write_buffer))?;
            if len == 0 {
                // 如果写入0字节，返回错误
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "write zero bytes",
                )));
            }
            this.write_buffer.advance(len);
        }
        Poll::Ready(Ok(()))
    }
}

impl<R> Stream for LengthDelimited<R>
where
    R: AsyncRead,
{
    type Item = Result<Bytes, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            match this.read_state {
                ReadState::ReadLength { buf, pos } => {
                    //先读取两个字节的长度
                    let n = ready!(this.inner.as_mut().poll_read(cx, &mut buf[*pos..]))?;
                    if *pos == 0 && n == 0 {
                        // 如果读取0字节，表示流已结束
                        return Poll::Ready(None);
                    } else if n == 0 {
                        // 如果读取0字节但不是开始位置，返回错误
                        return Poll::Ready(Some(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "unexpected end of stream",
                        ))));
                    }
                    *pos += n;
                    if *pos <= 1 {
                        continue; // 还没有读取完两个字节
                    }
                    // 读取完两个字节，解析长度
                    // 打印读取的长度buf
                    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
                    if len >= 1 {
                        *this.read_state = ReadState::ReadData { len, pos: 0 };
                        // 确保read_buffer有足够的空间
                        this.read_buffer.resize(len as usize, 0);
                    } else {
                        // 如果长度小于1，返回空的Bytes
                        *this.read_state = ReadState::default();
                        return Poll::Ready(Some(Ok(Bytes::new())));
                    }
                }
                ReadState::ReadData { len, pos } => {
                    let n = ready!(
                        this.inner
                            .as_mut()
                            .poll_read(cx, &mut this.read_buffer[*pos..])
                    )?;
                    if n == 0 {
                        // 如果读取0字节但还没有读取完数据，返回错误
                        return Poll::Ready(Some(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "unexpected end of stream",
                        ))));
                    }
                    *pos += n;
                    if *pos == *len as usize {
                        // 读取完数据，返回Bytes
                        let data = this.read_buffer.split_off(0).freeze();
                        *this.read_state = ReadState::default();
                        return Poll::Ready(Some(Ok(data)));
                    }
                }
            }
        }
    }
}

impl<R> Sink<Bytes> for LengthDelimited<R>
where
    R: AsyncWrite,
{
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // 缓冲区满了，先排空缓冲区
        if self.as_mut().project().write_buffer.len() >= MAX_FRAME_SIZE as usize {
            ready!(self.as_mut().poll_write_buffer(cx))?;
            debug_assert!(self.as_mut().project().write_buffer.is_empty());
        }
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: Bytes) -> Result<(), Self::Error> {
        let this = self.project();
        let len = match u32::try_from(item.len()) {
            Ok(len) if len <= MAX_FRAME_SIZE => len,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Maximum frame size exceeded.",
                ));
            }
        };
        this.write_buffer.reserve(len as usize + MAX_LENGTH_SIZE);
        this.write_buffer.put_u32(len);
        this.write_buffer.put(item);
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.as_mut().poll_write_buffer(cx))?;
        let this = self.project();
        debug_assert!(this.write_buffer.is_empty());
        this.inner.poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.as_mut().poll_write_buffer(cx))?;
        let this = self.project();
        debug_assert!(this.write_buffer.is_empty());
        this.inner.poll_close(cx)
    }
}

#[pin_project::pin_project]
#[derive(Debug)]
pub(crate) struct LengthDelimitedReader<R> {
    #[pin]
    inner: LengthDelimited<R>,
}

impl<R> LengthDelimitedReader<R> {
    pub(crate) fn into_inner(self) -> R {
        self.inner.into_inner()
    }
}

impl<R> Stream for LengthDelimitedReader<R>
where
    R: AsyncRead,
{
    type Item = Result<Bytes, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
}

impl<R> AsyncWrite for LengthDelimitedReader<R>
where
    R: AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let mut this = self.project().inner;
        ready!(this.as_mut().poll_write_buffer(cx))?;
        debug_assert!(this.write_buffer.is_empty());
        this.project().inner.poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let mut this = self.project().inner;
        ready!(this.as_mut().poll_write_buffer(cx))?;
        debug_assert!(this.write_buffer.is_empty());
        this.project().inner.poll_write_vectored(cx, bufs)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project().inner.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project().inner.poll_close(cx)
    }
}
