// SPDX-License-Identifier: GPL-3.0-or-later
//! One-use trusted Rust admission for the existing iOS document exporter. The
//! imported C bridge runs UIKit's presentation synchronously under Commit.
use super::Commit;
use std::{collections::HashMap, ffi::{c_char, c_void, CStr}, sync::{LazyLock, Mutex}};
static EXPORTS: LazyLock<Mutex<HashMap<String, Commit>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
unsafe extern "C" {
    fn kiwix_pdf_export_install(commit: unsafe extern "C" fn(*const c_char, *mut c_void, unsafe extern "C" fn(*mut c_void)) -> bool);
}
pub struct ExportAdmission(String);
impl ExportAdmission { pub fn token(&self) -> &str { &self.0 } }
impl Drop for ExportAdmission {
    fn drop(&mut self) { EXPORTS.lock().unwrap_or_else(|error| error.into_inner()).remove(&self.0); }
}
pub fn admit_export(commit: Commit) -> Result<ExportAdmission, String> {
    // Install through the imported C module, rather than leaving that module
    // with an unresolved Rust symbol in standalone Swift package policy tests.
    // SAFETY: the bridge stores only this static C-ABI function pointer,
    // synchronized by its lock, and invokes it synchronously with the live
    // Objective-C block and token that this trampoline requires.
    unsafe { kiwix_pdf_export_install(kiwix_pdf_commit_action); }
    let token = uuid::Uuid::new_v4().to_string();
    EXPORTS.lock().map_err(|_| "PDF export could not start. Try again")?.insert(token.clone(), commit);
    Ok(ExportAdmission(token))
}
unsafe extern "C" fn kiwix_pdf_commit_action(token: *const c_char, context: *mut c_void, action: unsafe extern "C" fn(*mut c_void)) -> bool {
    if token.is_null() || context.is_null() { return false; }
    // SAFETY: Swift's withCString keeps the NUL-terminated token readable
    // throughout the synchronous bridge call. Rust uses it only for the lookup.
    let Ok(token) = (unsafe { CStr::from_ptr(token) }).to_str() else { return false; };
    // Consume before calling UIKit. Duplicate/native reentrant presentations
    // cannot renew the authority or invoke the action a second time.
    let commit = EXPORTS.lock().ok().and_then(|mut jobs| jobs.remove(token));
    let Some(commit) = commit else { return false; };
    // SAFETY: the native caller invokes this trampoline only with the live
    // Objective-C block that `kiwix_pdf_export_commit` received; the action
    // runs synchronously and is never stored.
    commit(&mut || { unsafe { action(context) }; Ok(()) }).is_ok()
}
