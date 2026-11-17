// SPDX-License-Identifier: GPL-3.0-or-later
//! Application-owned Android operations remain callable without an Activity.
use jni::{objects::{GlobalRef, JObject, JString}, JNIEnv, JavaVM};
use serde_json::Value;
use std::sync::{Arc, Mutex};

struct Endpoint {
    vm: JavaVM,
    object: GlobalRef,
}
static INSTALLING: Mutex<Option<Arc<Endpoint>>> = Mutex::new(None);

#[no_mangle]
pub extern "system" fn Java_org_kiwix_mobile_KiwixMobilePlugin_installServiceEndpoint(
    mut env: JNIEnv<'_>, _plugin: JObject<'_>, endpoint: JObject<'_>,
) {
    let result = (|| -> Result<(), String> {
        let endpoint = Arc::new(Endpoint {
            vm: env.get_java_vm().map_err(|error| error.to_string())?,
            object: env.new_global_ref(endpoint).map_err(|error| error.to_string())?,
        });
        let mut slot = INSTALLING.lock().map_err(|_| "Native service endpoint unavailable")?;
        if slot.is_some() { return Err("Native service endpoint already installed".into()); }
        *slot = Some(endpoint);
        Ok(())
    })();
    if let Err(error) = result { let _ = env.throw_new("java/lang/IllegalStateException", error); }
}

impl Endpoint {
    fn call(&self, command: &str, payload: &Value) -> Result<Value, String> {
        let mut env = self.vm.attach_current_thread().map_err(|error| error.to_string())?;
        let result = (|| -> Result<Value, String> {
            let command = env.new_string(command).map_err(|error| error.to_string())?;
            let payload = env.new_string(payload.to_string()).map_err(|error| error.to_string())?;
            let response = env.call_method(self.object.as_obj(), "call", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                &[(&command).into(), (&payload).into()]).map_err(|error| error.to_string())?
                .l().map_err(|error| error.to_string())?;
            let response: String = env.get_string(&JString::from(response)).map_err(|error| error.to_string())?.into();
            let mut response: Value = serde_json::from_str(&response).map_err(|error| error.to_string())?;
            if let Some(error) = response.get("error").and_then(Value::as_str) { return Err(error.into()); }
            response.as_object_mut().and_then(|response| response.remove("value"))
                .ok_or_else(|| "Native service endpoint returned no completion".into())
        })();
        // A Java exception is reported, never left pending on a reused worker.
        if env.exception_check().unwrap_or(false) { let _ = env.exception_clear(); }
        result
    }
}

pub(crate) struct ServiceEndpoint {
    endpoint: Arc<Endpoint>,
}
impl ServiceEndpoint {
    pub(crate) fn installed() -> Result<Self, String> {
        let endpoint = INSTALLING.lock().map_err(|_| "Native service endpoint unavailable")?.take()
            .ok_or("Native service endpoint was not installed")?;
        Ok(Self { endpoint })
    }
    pub(crate) fn owns(command: &str, payload: &Value) -> bool {
        matches!(command, "activityState" | "finishActivity" | "retireWindow" | "openWindow"
            | "acquireExternalArchive" | "releaseExternalArchive" | "forgetExternalArchive"
            | "speechStop" | "speechRetire" | "speechPause" | "speechResume" | "speechPrevious"
            | "speechNext" | "speechRewind" | "speechForward" | "speechSeek"
            | "speechEngineStart" | "speechEngineUpdate" | "speechEngineStop"
            | "prepareExternalArchiveMove" | "deleteExternalArchiveMove"
            | "crashCapturePolicy" | "applyLocale"
            | "downloadNotifications" | "transferState" | "takeTransferEvents"
            | "acknowledgeTransferEvent" | "networkState" | "bindLocalSocket" | "isLocalPeer"
            | "wifiPerformance"
            | "nearbyBluetoothConnect" | "nearbyBluetoothSend" | "nearbyBluetoothDisconnect" | "nearbyBluetoothStop" | "nearbyBluetoothScan"
            | "nearbyJoinState" | "nearbyJoinBindSocket" | "nearbyJoinRelease" | "nearbyJoinCancel"
            | "nearbyRouteBindSocket" | "nearbyRouteState" | "nearbyRetire"
            | "hotspotQuicRoute" | "hotspotQuicState"
            | "registerListener" | "takeIncomingDocuments")
            || (command == "discoveryState" && payload.get("enabled") == Some(&Value::Bool(false)))
    }
    pub(crate) fn call_sync(&self, command: &str, payload: &Value) -> Result<Value, String> {
        self.endpoint.call(command, payload)
    }
    pub(crate) async fn call(&self, command: &'static str, payload: Value) -> Result<Value, String> {
        let endpoint = self.endpoint.clone();
        tauri::async_runtime::spawn_blocking(move || endpoint.call(command, &payload)).await.map_err(|error| error.to_string())?
    }
}
