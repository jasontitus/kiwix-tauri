#[macro_use]
mod utils;

test_cases!(gzip);

#[allow(unused)]
use utils::{algos::gzip::sync, InputStream};

#[cfg(feature = "futures-io")]
use utils::algos::gzip::futures::bufread;

#[allow(unused)]
fn compress_with_header(data: &[u8]) -> Vec<u8> {
    use flate2::{Compression, GzBuilder};
    use std::io::Write;

    let mut bytes = Vec::new();
    {
        let mut gz = GzBuilder::new()
            .filename("hello_world.txt")
            .comment("test file, please delete")
            .extra(vec![1, 2, 3, 4])
            .write(&mut bytes, Compression::fast());

        gz.write_all(data).unwrap();
    }

    bytes
}

#[test]
#[ntest::timeout(1000)]
#[cfg(feature = "futures-io")]
fn gzip_bufread_decompress_with_extra_header() {
    let bytes = compress_with_header(&[1, 2, 3, 4, 5, 6]);

    let input = InputStream::from(vec![bytes]);
    let output = bufread::decompress(bufread::from(&input));

    assert_eq!(output, &[1, 2, 3, 4, 5, 6][..]);
}

#[test]
#[ntest::timeout(1000)]
#[cfg(feature = "futures-io")]
fn gzip_bufread_chunks_decompress_with_extra_header() {
    let bytes = compress_with_header(&[1, 2, 3, 4, 5, 6]);

    let input = InputStream::from(bytes.chunks(2));
    let output = bufread::decompress(bufread::from(&input));

    assert_eq!(output, &[1, 2, 3, 4, 5, 6][..]);
}

#[test]
#[ntest::timeout(1000)]
#[cfg(feature = "futures-io")]
fn gzip_bufread_chunks_compress_flushes_when_reader_pending() {
    use crate::utils::block_on;
    use async_compression::futures::bufread::GzipEncoder;
    use futures::AsyncRead;
    use futures::AsyncReadExt;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll};

    struct Input {
        remaining: usize,
        chunks_sent: Arc<AtomicUsize>,
        chunks_received: Arc<AtomicUsize>,
    }

    impl AsyncRead for Input {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.chunks_received.load(Ordering::Relaxed)
                < self.chunks_sent.load(Ordering::Relaxed)
            {
                Poll::Pending
            } else {
                let bytes = b"X".repeat(buf.len().min(64).min(self.remaining));
                buf[..bytes.len()].copy_from_slice(&bytes);
                let this = self.get_mut();
                this.remaining -= bytes.len();
                this.chunks_sent.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(Ok(bytes.len()))
            }
        }
    }

    let chunks_sent = Arc::new(AtomicUsize::new(0));
    let chunks_received = Arc::new(AtomicUsize::new(0));

    let input = futures::io::BufReader::new(Input {
        remaining: 4 * 1024,
        chunks_sent: Arc::clone(&chunks_sent),
        chunks_received: Arc::clone(&chunks_received),
    });

    let mut encoder = GzipEncoder::new(input);

    let mut encoded_buffer: [u8; 64] = [0; 64];

    block_on(async {
        while let Ok(read) = encoder.read(&mut encoded_buffer).await {
            if read == 0 {
                break;
            }
            chunks_received.fetch_add(1, Ordering::Relaxed);
        }
    });

    assert_eq!(
        chunks_sent.load(Ordering::Relaxed),
        chunks_received.load(Ordering::Relaxed)
    );
}

#[test]
#[ntest::timeout(1000)]
#[cfg(feature = "futures-io")]
fn gzip_bufread_chunks_decompress_without_footer_emits_all_payload() {
    use flate2::bufread::GzDecoder;
    use std::io::Read;

    let mut bytes = compress_with_header(&[1, 2, 3, 4, 5, 6]);

    // Remove the footer.
    bytes.truncate(bytes.len() - 8);

    let mut decoder = GzDecoder::new(bytes.as_slice());

    let mut output = vec![];
    let result = decoder.read_to_end(&mut output);

    assert!(result.is_err());
    assert_eq!(output, &[1, 2, 3, 4, 5, 6][..]);
}

/// Yields `data` once and then stays pending, modelling a live HTTP body that neither ends nor
/// sends more bytes.
#[cfg(feature = "futures-io")]
struct StalledReader {
    data: Vec<u8>,
    pos: usize,
}

#[cfg(feature = "futures-io")]
impl futures::io::AsyncRead for StalledReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if this.pos == this.data.len() {
            return std::task::Poll::Pending;
        }
        let n = (this.data.len() - this.pos).min(buf.len());
        buf[..n].copy_from_slice(&this.data[this.pos..this.pos + n]);
        this.pos += n;
        std::task::Poll::Ready(Ok(n))
    }
}

#[cfg(feature = "futures-io")]
impl futures::io::AsyncBufRead for StalledReader {
    fn poll_fill_buf(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<&[u8]>> {
        let this = self.get_mut();
        if this.pos == this.data.len() {
            return std::task::Poll::Pending;
        }
        std::task::Poll::Ready(Ok(&this.data[this.pos..]))
    }

    fn consume(self: std::pin::Pin<&mut Self>, amt: usize) {
        let this = self.get_mut();
        this.pos = (this.pos + amt).min(this.data.len());
    }
}

/// A gzip member followed by bytes too short to complete a header, on a stream that never ends.
/// The bytes already contradict the magic, so decoding must fail instead of waiting for more.
#[test]
#[ntest::timeout(1000)]
#[cfg(feature = "futures-io")]
fn bufread_multiple_members_rejects_short_invalid_header() {
    use futures::{executor::block_on, io::AsyncReadExt as _};

    let mut compressed = sync::compress(&[1, 2, 3, 4, 5, 6]);
    compressed.extend_from_slice(b"ZZ");

    let mut decoder = bufread::Decoder::new(StalledReader {
        data: compressed,
        pos: 0,
    });
    decoder.multiple_members(true);

    let mut output = Vec::new();
    assert!(block_on(decoder.read_to_end(&mut output)).is_err());
}

/// Sets FHCRC on a gzip member whose header is `header_len` bytes long and inserts the CRC16.
#[allow(unused)]
fn with_header_crc(mut bytes: Vec<u8>, header_len: usize) -> Vec<u8> {
    bytes[3] |= 0b0000_0010;
    let mut crc = flate2::Crc::new();
    crc.update(&bytes[..header_len]);
    let crc16 = crc.sum().to_le_bytes();
    bytes.splice(header_len..header_len, crc16[..2].iter().copied());
    bytes
}

#[test]
#[ntest::timeout(1000)]
#[cfg(feature = "futures-io")]
fn gzip_bufread_decompress_with_header_crc() {
    let bytes = with_header_crc(sync::compress(&[1, 2, 3, 4, 5, 6]), 10);
    assert_eq!(sync::decompress(&bytes), &[1, 2, 3, 4, 5, 6][..]);

    let input = InputStream::from(vec![bytes]);
    let output = bufread::decompress(bufread::from(&input));

    assert_eq!(output, &[1, 2, 3, 4, 5, 6][..]);
}

#[test]
#[ntest::timeout(1000)]
#[cfg(feature = "futures-io")]
fn gzip_bufread_chunks_decompress_with_extra_header_and_header_crc() {
    let header_len = 10 + 2 + 4 + "hello_world.txt\0".len() + "test file, please delete\0".len();
    let bytes = with_header_crc(compress_with_header(&[1, 2, 3, 4, 5, 6]), header_len);
    assert_eq!(sync::decompress(&bytes), &[1, 2, 3, 4, 5, 6][..]);

    let input = InputStream::from(bytes.chunks(1));
    let output = bufread::decompress(bufread::from(&input));

    assert_eq!(output, &[1, 2, 3, 4, 5, 6][..]);
}

#[test]
#[ntest::timeout(1000)]
#[cfg(feature = "futures-io")]
fn gzip_bufread_decompress_rejects_wrong_header_crc() {
    use futures::{executor::block_on, io::AsyncReadExt as _};

    let mut bytes = with_header_crc(sync::compress(&[1, 2, 3, 4, 5, 6]), 10);
    bytes[10] ^= 0xff;

    let input = InputStream::from(vec![bytes]);
    let mut decoder = bufread::Decoder::new(bufread::from(&input));

    let mut output = Vec::new();
    let err = block_on(decoder.read_to_end(&mut output)).unwrap_err();
    assert_eq!(err.to_string(), "CRC computed for header does not match");
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn tokio_read_resumes_after_cancelling_a_pending_poll() {
    use async_compression::tokio::bufread::GzipDecoder;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

    let payload: Vec<_> = (0..(140 * 1024)).map(|i| (i % 251) as u8).collect();
    let compressed = sync::compress(&payload);
    let (mut producer, input) = tokio::io::duplex(16);
    let mut decoder = GzipDecoder::new(BufReader::new(input));
    let mut first = [0; 17];
    let mut pending = Box::pin(decoder.read(&mut first));
    assert!(futures::poll!(pending.as_mut()).is_pending());
    drop(pending);
    assert_eq!(first, [0; 17]);

    let sender = tokio::spawn(async move {
        for chunk in compressed.chunks(3) {
            producer.write_all(chunk).await.unwrap();
        }
        producer.shutdown().await.unwrap();
    });
    let mut output = Vec::with_capacity(payload.len());
    decoder.read_to_end(&mut output).await.unwrap();
    sender.await.unwrap();
    assert_eq!(output, payload);
}

#[cfg(feature = "tokio")]
mod input_error_tests {
    use super::sync;
    use async_compression::tokio::bufread::{GzipDecoder, GzipEncoder};
    use std::{
        io::{self, ErrorKind},
        pin::Pin,
        task::{ready, Context, Poll},
    };
    use tokio::io::{AsyncBufRead, AsyncRead, AsyncReadExt, ReadBuf};

    struct ErrorBetweenChunks<'a> {
        first: &'a [u8],
        error: Option<io::Error>,
        rest: &'a [u8],
        polls: usize,
    }

    impl<'a> ErrorBetweenChunks<'a> {
        fn new(input: &'a [u8], split: usize, kind: ErrorKind) -> Self {
            let (first, rest) = input.split_at(split);
            Self {
                first,
                rest,
                error: Some(io::Error::new(kind, "input interrupted between chunks")),
                polls: 0,
            }
        }
    }

    impl AsyncRead for ErrorBetweenChunks<'_> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            output: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let input = ready!(self.as_mut().poll_fill_buf(cx))?;
            let count = input.len().min(output.remaining());
            output.put_slice(&input[..count]);
            self.consume(count);
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncBufRead for ErrorBetweenChunks<'_> {
        fn poll_fill_buf(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<io::Result<&[u8]>> {
            let this = self.get_mut();
            this.polls += 1;
            if !this.first.is_empty() {
                return Poll::Ready(Ok(this.first));
            }
            if let Some(error) = this.error.take() {
                return Poll::Ready(Err(error));
            }
            Poll::Ready(Ok(this.rest))
        }

        fn consume(self: Pin<&mut Self>, count: usize) {
            let this = self.get_mut();
            let active = if this.first.is_empty() {
                &mut this.rest
            } else {
                &mut this.first
            };
            *active = &active[count..];
        }
    }

    #[tokio::test]
    async fn decoder_delivers_output_before_input_error_and_can_resume() {
        let payload = b"article before gzip footer";
        let compressed = sync::compress(payload);
        let input = ErrorBetweenChunks::new(
            &compressed,
            compressed.len() - 8,
            ErrorKind::ConnectionReset,
        );
        let mut decoder = GzipDecoder::new(input);

        let mut buffer = [0; 1024];
        let count = decoder.read(&mut buffer).await.unwrap();
        assert_eq!(&buffer[..count], payload);
        assert_eq!(decoder.get_ref().polls, 2);

        let mut output = buffer[..count].to_vec();
        let error = decoder.read_to_end(&mut output).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);
        assert_eq!(error.to_string(), "input interrupted between chunks");
        assert_eq!(output, payload);
        assert_eq!(decoder.get_ref().polls, 2);

        assert_eq!(decoder.read_to_end(&mut output).await.unwrap(), 0);
        assert_eq!(output, payload);
    }

    #[tokio::test]
    async fn encoder_delivers_output_then_replays_interruption_once() {
        let payload = b"first chunk and second chunk";
        let input = ErrorBetweenChunks::new(payload, 11, ErrorKind::Interrupted);
        let mut encoder = GzipEncoder::new(input);

        let mut buffer = [0; 1024];
        let count = encoder.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        assert_eq!(encoder.get_ref().polls, 2);
        let mut output = buffer[..count].to_vec();

        assert_eq!(encoder.read(&mut []).await.unwrap(), 0);
        let error = encoder.read(&mut buffer).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);
        assert_eq!(error.to_string(), "input interrupted between chunks");
        assert_eq!(encoder.get_ref().polls, 2);

        encoder.read_to_end(&mut output).await.unwrap();
        assert_eq!(sync::decompress(&output), payload);
    }

    #[tokio::test]
    async fn decoder_reports_error_without_output_and_keeps_decoder_state() {
        let payload = b"payload after retry";
        let compressed = sync::compress(payload);
        let input = ErrorBetweenChunks::new(&compressed, 0, ErrorKind::Interrupted);
        let mut decoder = GzipDecoder::new(input);
        let mut buf = [0; 128];
        let error = decoder.read(&mut buf).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);
        assert_eq!(decoder.get_ref().polls, 1);

        let mut output = Vec::new();
        decoder.read_to_end(&mut output).await.unwrap();
        assert_eq!(output, payload);
    }
}
