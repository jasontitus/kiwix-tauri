// SPDX-License-Identifier: GPL-3.0-or-later
//! Match Apple location results to a live, archive-scoped request.
#![deny(unsafe_code)]

use crate::{
    apple_replies::{Replies, Reply},
    permissions::{Coordinates, LocationError, Position},
};
use objc2_foundation::NSData;
use std::sync::LazyLock;

type LocationResult = Result<Position, LocationError>;

static REPLIES: LazyLock<Replies<LocationResult>> = LazyLock::new(Replies::default);

#[repr(C)]
struct NativePosition {
    latitude: f64,
    longitude: f64,
    accuracy: f64,
    altitude: f64,
    altitude_accuracy: f64,
    heading: f64,
    speed: f64,
    timestamp: f64,
}

#[allow(unsafe_code, reason = "declare the verified Objective-C++ ABI in one place")]
#[deny(improper_ctypes)]
unsafe extern "C" {
    // Native copies only values and static function pointers into the queued
    // operation. Rust borrows native-owned immutable diagnostics during the callback.
    safe fn kiwix_location_start(
        token: u64,
        high_accuracy: bool,
        timeout_ms: u64,
        is_live: extern "C" fn(u64) -> bool,
        callback: extern "C" fn(u64, i32, NativePosition, Option<&NSData>),
    );
    safe fn kiwix_location_cancel(token: u64);
}

pub(crate) fn register() -> Option<Reply<'static, LocationResult>> {
    REPLIES.register()
}

pub(crate) fn start(token: u64, high_accuracy: bool, timeout_ms: u64) {
    kiwix_location_start(token, high_accuracy, timeout_ms, is_live, receive);
}

pub(crate) fn cancel(token: u64) {
    // Retire the Rust waiter before waiting for main-queue cleanup. Session
    // consent is rechecked independently by SensorManager before delivery.
    REPLIES.finish(token, || Err(LocationError::cancelled()));
    kiwix_location_cancel(token);
}

extern "C" fn is_live(token: u64) -> bool {
    REPLIES.is_live(token)
}

extern "C" fn receive(
    token: u64,
    code: i32,
    position: NativePosition,
    message: Option<&NSData>,
) {
    REPLIES.finish(token, || {
        if code == 0 {
            return position.checked();
        }
        Err(LocationError {
            code: match code {
                1 => 1,
                3 => 3,
                _ => 2,
            },
            message: message
                .and_then(|data| String::from_utf8(data.to_vec()).ok())
                .unwrap_or_else(|| "Location unavailable".into()),
            cancelled: false,
        })
    });
}

impl NativePosition {
    fn checked(self) -> LocationResult {
        if !(-90.0..=90.0).contains(&self.latitude)
            || !(-180.0..=180.0).contains(&self.longitude)
            || !self.accuracy.is_finite()
            || self.accuracy < 0.0
            || !self.timestamp.is_finite()
        {
            return Err(LocationError::unavailable(
                "Native location provider returned an invalid position",
            ));
        }

        let nonnegative = |value: f64| (value.is_finite() && value >= 0.0).then_some(value);
        Ok(Position {
            timestamp: self.timestamp,
            coords: Coordinates {
                latitude: self.latitude,
                longitude: self.longitude,
                accuracy: self.accuracy,
                altitude: self.altitude.is_finite().then_some(self.altitude),
                altitude_accuracy: nonnegative(self.altitude_accuracy),
                heading: nonnegative(self.heading),
                speed: nonnegative(self.speed),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position() -> NativePosition {
        NativePosition {
            latitude: 37.4,
            longitude: -122.1,
            accuracy: 10.0,
            altitude: -5.0,
            altitude_accuracy: 3.0,
            heading: f64::NAN,
            speed: -1.0,
            timestamp: 1234.0,
        }
    }

    #[tokio::test]
    async fn only_the_live_request_gets_the_first_position() {
        let retired = register().unwrap();
        let retired_token = retired.id();
        let live = register().unwrap();
        let live_token = live.id();
        cancel(retired_token);
        assert!(!is_live(retired_token));
        assert!(retired.wait().await.unwrap().unwrap_err().cancelled);
        receive(retired_token, 0, position(), None);
        assert!(is_live(live_token));
        receive(live_token, 0, position(), None);
        receive(live_token, 2, position(), None);
        assert!(!is_live(live_token));

        let fix = live.wait().await.unwrap().unwrap();
        assert_eq!(fix.timestamp, 1234.0);
        assert_eq!(fix.coords.latitude, 37.4);
        assert_eq!(fix.coords.longitude, -122.1);
        assert_eq!(fix.coords.accuracy, 10.0);
        assert_eq!(fix.coords.altitude, Some(-5.0));
        assert_eq!(fix.coords.altitude_accuracy, Some(3.0));
        assert_eq!(fix.coords.heading, None);
        assert_eq!(fix.coords.speed, None);
    }

    #[tokio::test]
    async fn callback_copies_the_diagnostic_and_rejects_invalid_fixes() {
        let failure = register().unwrap();
        let diagnostic = NSData::with_bytes("Position indisponible — réessayer".as_bytes());
        receive(failure.id(), 3, position(), Some(&diagnostic));
        drop(diagnostic);
        let failure = failure.wait().await.unwrap().unwrap_err();
        assert_eq!(failure.code, 3);
        assert_eq!(failure.message, "Position indisponible — réessayer");
        assert!(!failure.cancelled);

        let invalid_diagnostic = NSData::with_bytes(&[0xff]);
        for diagnostic in [None, Some(&*invalid_diagnostic)] {
            let missing = register().unwrap();
            receive(missing.id(), 2, position(), diagnostic);
            assert_eq!(missing.wait().await.unwrap().unwrap_err().message, "Location unavailable");
        }

        let invalid = register().unwrap();
        let mut fix = position();
        fix.latitude = f64::NAN;
        receive(invalid.id(), 0, fix, None);
        let invalid = invalid.wait().await.unwrap().unwrap_err();
        assert_eq!(invalid.code, 2);
        assert!(invalid.message.contains("invalid position"));
    }
}
