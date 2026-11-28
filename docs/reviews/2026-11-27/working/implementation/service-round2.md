The features lane's `finish_voice_job` edit is in place and I am making no further edits. `service.rs:3782` reads `"state": if success { "installed" } else if control.cancelled() { "cancelled" } else { "error" }`, and `kokoro.rs:191` has the `DownloadControl::cancelled()` accessor it calls.

That also lands the FEAT-9 handoff from my earlier reports, so it no longer needs to be listed as open.

My round-2 report stands as sent. I did not re-run the build, tests or clippy after that edit appeared. Earlier in this round I edited `service.rs` with whole-file Python rewrites; the coordinator confirmed at the start of the round that those dropped no other lane's edit.