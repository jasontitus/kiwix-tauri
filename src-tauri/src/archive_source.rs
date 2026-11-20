// SPDX-License-Identifier: GPL-3.0-or-later
//! Read-only source admission. Provider authority never becomes a filesystem path
//! exception; native leases outlive every libzim handle made from their descriptors.
#![forbid(unsafe_code)]
use crate::{archive::Archive, library_files::PinnedArchive, store::Book};
use std::fs::File;

pub(crate) enum RetainedSource {
    Filesystem(PinnedArchive),
    #[cfg(mobile)]
    Provider(crate::provider_reference::ProviderFiles),
}
impl RetainedSource {
    pub(crate) fn files(&self) -> &[(String, File)] {
        match self {
            Self::Filesystem(source) => &source.files,
            #[cfg(mobile)]
            Self::Provider(source) => &source.files,
        }
    }
    pub(crate) fn files_mut(&mut self) -> &mut [(String, File)] {
        match self {
            Self::Filesystem(source) => &mut source.files,
            #[cfg(mobile)]
            Self::Provider(source) => &mut source.files,
        }
    }
    pub(crate) fn unchanged(&self) -> Result<(), String> {
        match self {
            Self::Filesystem(source) => crate::library_files::retained_source_unchanged(source),
            #[cfg(mobile)]
            Self::Provider(_) => Ok(()),
        }
    }
    pub(crate) fn open(&self) -> Result<Archive, String> {
        match self {
            Self::Filesystem(source) => Archive::open_files(&source.path, &source.files),
            #[cfg(mobile)]
            Self::Provider(source) => source.open(),
        }
    }
    pub(crate) fn identity(&self) -> Result<crate::integrity::ContentIdentity, String> {
        crate::integrity::ContentIdentity::from_files(self.files())
    }
}
pub(crate) fn retain(book: &Book) -> Result<RetainedSource, String> {
    if let Some(reference) = &book.source_reference {
        reference.validate_book(book)?;
        #[cfg(mobile)]
        return crate::provider_reference::retain_registered(reference).map(RetainedSource::Provider);
        #[cfg(not(mobile))]
        return Err("External document-provider references require their original mobile platform; select the archive again on this device".into());
    }
    crate::library_files::retain_read_archive_files(book).map(RetainedSource::Filesystem)
}
