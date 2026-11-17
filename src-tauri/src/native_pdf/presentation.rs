// SPDX-License-Identifier: GPL-3.0-or-later
//! Trusted service-owned presentation metadata, separate from archive DOM data.
pub struct Presentation {
    // Linux/Android consume the suggested filename; Apple/Windows retain the
    // same trusted presentation contract without reading this field.
    #[cfg_attr(any(target_os = "macos", target_os = "ios", target_os = "windows"), allow(dead_code))]
    pub name: String,
    #[cfg(target_os = "android")]
    pub labels: serde_json::Value,
}

#[cfg(target_os = "android")]
pub(super) fn android_share_payload(path: &std::path::Path, name: &str, job: &str, labels: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "path":path,"mimeType":"application/pdf","suggestedName":name,
        "open":true,"share":true,"pdfJobId":job,"labels":labels
    })
}

