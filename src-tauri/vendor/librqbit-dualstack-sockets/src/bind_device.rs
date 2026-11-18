#[cfg(test)]
pub(crate) mod tests;

use crate::Error;
use std::{ffi::CString, num::NonZeroU32, str::FromStr};

#[derive(Debug, Clone)]
pub struct BindDevice {
    #[allow(unused)]
    index: NonZeroU32,
    #[allow(unused)]
    name: CString,
}

impl BindDevice {
    #[cfg(not(windows))]
    pub fn new_from_name(name: &str) -> crate::Result<Self> {
        let name = CString::new(name).map_err(|_| Error::BindDeviceInvalid)?;

        let index = nix::net::if_::if_nametoindex(name.as_c_str())
            .map_err(|error| Error::BindDeviceInvalidError(error.into()))?;
        let index = NonZeroU32::new(index).ok_or(Error::BindDeviceInvalid)?;
        Ok(Self { index, name })
    }

    #[cfg(windows)]
    pub fn new_from_name(_name: &str) -> crate::Result<Self> {
        Err(Error::BindDeviceNotSupported)
    }

    pub fn index(&self) -> NonZeroU32 {
        self.index
    }

    pub fn name(&self) -> &str {
        self.name
            .to_str()
            .expect("interface name was created from &str")
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    pub fn bind_sref(&self, sref: &socket2::Socket, is_v6: bool) -> crate::Result<()> {
        if is_v6 {
            sref.bind_device_by_index_v6(Some(self.index))
                .map_err(Error::BindDeviceSetDeviceError)
        } else {
            sref.bind_device_by_index_v4(Some(self.index))
                .map_err(Error::BindDeviceSetDeviceError)
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", windows)))]
    pub fn bind_sref(&self, sref: &socket2::Socket, _is_v6: bool) -> crate::Result<()> {
        let name = self.name.as_bytes_with_nul();
        sref.bind_device(Some(name))
            .map_err(Error::BindDeviceSetDeviceError)
    }

    #[cfg(windows)]
    pub fn bind_sref(&self, _sref: &socket2::Socket, _is_v6: bool) -> crate::Result<()> {
        Err(Error::BindDeviceNotSupported)
    }
}

impl FromStr for BindDevice {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new_from_name(s)
    }
}
