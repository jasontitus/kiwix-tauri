mod decoder;
mod encoder;

pub(crate) use decoder::*;
pub(crate) use encoder::*;

use std::{io, panic::AssertUnwindSafe};

#[derive(Debug, Default)]
pub(crate) struct InputError(Option<AssertUnwindSafe<io::Error>>);

impl InputError {
    pub(crate) fn take(&mut self) -> Option<io::Error> {
        self.0.take().map(|error| error.0)
    }

    pub(crate) fn after_output(&mut self, error: io::Error, written: usize) -> io::Result<()> {
        if written == 0 {
            return Err(error);
        }
        // A successful short read reports the bytes already delivered. Report
        // this exact input error on the next read before resuming the codec.
        self.0 = Some(AssertUnwindSafe(error));
        Ok(())
    }
}
