// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! The acquisition pipeline consumes either Internet HTTP or the actual QUIC library stream.
use bytes::Bytes;
use futures_util::{stream, Stream};
use reqwest::{header::HeaderMap, StatusCode};

pub(crate) enum TransferResponse {
    Http(reqwest::Response),
    Quic(crate::localswarm::library::LibraryResponse),
}

impl std::fmt::Debug for TransferResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct(match self { Self::Http(_) => "Http", Self::Quic(_) => "Quic" })
            .field("status", &self.status()).finish_non_exhaustive()
    }
}

impl TransferResponse {
    pub(crate) fn status(&self) -> StatusCode {
        match self { Self::Http(response) => response.status(), Self::Quic(response) => response.status() }
    }

    pub(crate) fn headers(&self) -> &HeaderMap {
        match self { Self::Http(response) => response.headers(), Self::Quic(response) => response.headers() }
    }

    pub(crate) async fn chunk(&mut self) -> Result<Option<Bytes>, String> {
        match self {
            Self::Http(response) => response.chunk().await.map_err(|error| error.to_string()),
            Self::Quic(response) => response.chunk().await,
        }
    }

    /// Keep existing bounded readers and bandwidth accounting shared by both transports.
    pub(crate) fn bytes_stream(self) -> impl Stream<Item = Result<Bytes, String>> + Send {
        stream::try_unfold(self, |mut response| async move {
            Ok(response.chunk().await?.map(|bytes| (bytes, response)))
        })
    }
}
