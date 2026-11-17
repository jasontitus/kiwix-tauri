// SPDX-License-Identifier: GPL-3.0-or-later
//! Native document/lifecycle adapter. Only the trusted application Rust layer
//! invokes platform methods; archive pages receive no plugin command surface.
use serde_json::{json, Value};
use tauri::{ipc::{Channel, InvokeResponseBody}, plugin::{Builder, PluginHandle, TauriPlugin}, Manager, Runtime};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_kiwix_mobile);

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub mod android_pdf;

pub struct Mobile<R: Runtime> {
    handle: PluginHandle<R>,
    #[cfg(target_os = "android")]
    service: android::ServiceEndpoint,
}

impl<R: Runtime> Mobile<R> {
    pub async fn call(&self, command: &'static str, payload: Value) -> Result<Value, String> {
        #[cfg(target_os = "android")]
        {
            if android::ServiceEndpoint::owns(command, &payload) { return self.service.call(command, payload).await; }
            return self.handle.run_mobile_plugin_async(command, payload).await.map_err(|error| error.to_string());
        }
        #[cfg(target_os = "ios")]
        self.handle.run_mobile_plugin_async(command, payload).await.map_err(|error| error.to_string())
    }

    /// Internal Rust-only synchronous bridge for seekable descriptor admission.
    /// Call on a blocking worker, never the UIKit/Android UI thread: native
    /// provider acquisition performs coordinated I/O on its document worker.
    pub fn call_sync(&self, command: &'static str, payload: Value) -> Result<Value, String> {
        #[cfg(target_os = "android")]
        if android::ServiceEndpoint::owns(command, &payload) { return self.service.call_sync(command, &payload); }
        self.handle.run_mobile_plugin(command, payload).map_err(|error| error.to_string())
    }

    #[cfg(target_os = "android")]
    pub fn storage_root(&self) -> Result<std::path::PathBuf, String> {
        let response: Value = self.handle.run_mobile_plugin("storageRoot", json!({}))
            .map_err(|error| error.to_string())?;
        let path = response.get("path").and_then(Value::as_str)
            .ok_or("Native Android storage root is missing")?;
        let path = std::path::PathBuf::from(path);
        if !path.is_absolute() || !path.is_dir() {
            return Err("Native Android storage root must be an existing absolute directory".into());
        }
        Ok(path)
    }

    pub async fn listen(&self, event: &'static str, callback: impl Fn(Value) + Send + Sync + 'static) -> Result<(), String> {
        let channel = Channel::<Value>::new(move |body| {
            match body {
                InvokeResponseBody::Json(text) => callback(serde_json::from_str(&text)?),
                InvokeResponseBody::Raw(_) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Native mobile events must be JSON").into()),
            }
            Ok(())
        });
        self.call("registerListener", json!({"event": event, "handler": channel})).await?;
        Ok(())
    }
}

pub trait MobileExt<R: Runtime> {
    fn kiwix_mobile(&self) -> &Mobile<R>;
}
impl<R: Runtime, T: Manager<R>> MobileExt<R> for T {
    fn kiwix_mobile(&self) -> &Mobile<R> { self.state::<Mobile<R>>().inner() }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("kiwix-mobile").setup(|app, api| {
        #[cfg(target_os = "android")]
        let handle = api.register_android_plugin("org.kiwix.mobile", "KiwixMobilePlugin")?;
        #[cfg(target_os = "ios")]
        let handle = api.register_ios_plugin(init_plugin_kiwix_mobile)?;
        #[cfg(target_os = "android")]
        {
            let _: Value = handle.run_mobile_plugin("installApplicationResources", json!({}))?;
            app.manage(Mobile { handle, service: android::ServiceEndpoint::installed()? });
        }
        #[cfg(target_os = "ios")]
        app.manage(Mobile { handle });
        Ok(())
    }).build()
}
