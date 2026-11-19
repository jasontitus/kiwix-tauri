mod decoder;
mod encoder;

pub use self::{decoder::Decoder, encoder::Encoder};

use crate::core::util::WriteBuffer;
use std::{io::Result, task::Poll};
use tokio::io::ReadBuf;

fn poll_read(
    buf: &mut ReadBuf<'_>,
    do_poll_read: impl FnOnce(&mut WriteBuffer<'_>) -> Poll<Result<()>>,
) -> Poll<Result<()>> {
    if buf.remaining() == 0 {
        return Poll::Ready(Ok(()));
    }

    // AsyncRead can return a partial read. Avoid eagerly zeroing an arbitrarily
    // large caller buffer before the input is ready.
    const MAX_OUTPUT_PER_POLL: usize = 64 * 1024;
    let limit = buf.remaining().min(MAX_OUTPUT_PER_POLL);
    let mut output = WriteBuffer::new_initialized(buf.initialize_unfilled_to(limit));
    let res = do_poll_read(&mut output);
    let written = output.written_len();
    buf.advance(written);

    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::MaybeUninit;

    #[test]
    fn pending_and_partial_poll_preserve_the_filled_prefix() {
        let mut storage = vec![MaybeUninit::uninit(); 128 * 1024];
        let mut buf = ReadBuf::uninit(&mut storage);
        buf.put_slice(b"prefix");
        let result = poll_read(&mut buf, |output| {
            assert_eq!(output.capacity(), 64 * 1024);
            Poll::Pending
        });
        assert!(result.is_pending());
        assert_eq!(buf.filled(), b"prefix");
        assert_eq!(buf.initialized().len(), 64 * 1024 + 6);

        let result = poll_read(&mut buf, |output| {
            output.initialize_unwritten()[..3].copy_from_slice(b"new");
            output.advance(3);
            Poll::Ready(Ok(()))
        });
        assert!(matches!(result, Poll::Ready(Ok(()))));
        assert_eq!(buf.filled(), b"prefixnew");
        assert_eq!(buf.initialized().len(), 64 * 1024 + 6);
    }

    #[test]
    fn full_output_skips_the_codec() {
        let mut storage = [42];
        let mut buf = ReadBuf::new(&mut storage);
        buf.advance(1);
        let result = poll_read(&mut buf, |_| panic!("full buffer reached the codec"));
        assert!(matches!(result, Poll::Ready(Ok(()))));
        assert_eq!(buf.filled(), &[42]);
    }
}
