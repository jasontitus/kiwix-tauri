// SPDX-License-Identifier: GPL-3.0-or-later
//! Rust-only admission for the public Android print adapter. No renderer command
//! can create or renew an admission. JNI checks run synchronously at handoff.
use jni::{JNIEnv, objects::{JClass, JObject, JString}, sys::jboolean};
use std::{collections::HashMap, sync::{Arc, LazyLock, Mutex}};
type Admission = Arc<dyn Fn(bool) -> bool + Send + Sync>;
pub type Commit = Arc<dyn Fn(&mut dyn FnMut() -> Result<(), String>) -> Result<(), String> + Send + Sync>;
#[derive(Clone)]
struct Job { check: Admission, commit: Commit }
static JOBS: LazyLock<Mutex<HashMap<String, Job>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
fn jobs() -> &'static Mutex<HashMap<String, Job>> {
    &JOBS
}
pub struct PdfAdmission(String);
impl Drop for PdfAdmission { fn drop(&mut self) { if let Ok(mut jobs) = jobs().lock() { jobs.remove(&self.0); } } }
pub fn admit(id: String, authority: Admission, commit: Commit) -> Result<PdfAdmission, String> {
    jobs().lock().map_err(|_| "Native PDF authority registry unavailable")?.insert(id.clone(), Job { check: authority, commit });
    Ok(PdfAdmission(id))
}
#[no_mangle]
pub extern "system" fn Java_org_kiwix_mobile_NativeArticlePdf_authorityCurrent(mut env: JNIEnv, _class: JClass, id: JString, handed_off: jboolean) -> jboolean {
    let Ok(id) = env.get_string(&id) else { return 0; };
    let id: String = id.into();
    let job = jobs().lock().ok().and_then(|jobs| jobs.get(&id).cloned());
    job.is_some_and(|job| (job.check)(handed_off != 0)) as jboolean
}
#[no_mangle]
pub extern "system" fn Java_org_kiwix_mobile_NativeArticlePdf_commitHandoff(mut env: JNIEnv, _class: JClass, id: JString, action: JObject) -> jboolean {
    let Ok(id) = env.get_string(&id) else { return 0; };
    let id: String = id.into();
    let job = jobs().lock().ok().and_then(|jobs| jobs.get(&id).cloned());
    let Some(job) = job else { return 0; };
    // Parent's short commit holds retirement guards across this synchronous
    // Java action, never across loading or the user's print-dialog lifetime.
    (job.commit)(&mut || env.call_method(&action, "run", "()V", &[])
        .map(|_| ()).map_err(|error| format!("Android PDF handoff failed: {error}"))).is_ok() as jboolean
}
