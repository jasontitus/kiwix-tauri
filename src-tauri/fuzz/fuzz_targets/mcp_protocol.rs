#![no_main]
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

#[path = "../../../kiwix-mcp/src/protocol.rs"]
mod protocol;

struct Backend;
impl protocol::Backend for Backend {
    fn call(&mut self, _tool: &str, _arguments: Value) -> Result<Value, String> {
        Err("fuzz backend".into())
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return; };
    let mut session = protocol::Session::default();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let _ = session.handle(line, &mut Backend);
    }
});
