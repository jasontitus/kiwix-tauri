//! `kiwix-mcp`: lets local AI agents search and read the user's offline Kiwix
//! library through the Model Context Protocol (stdio transport).
//!
//! MCP clients start this program and talk JSON-RPC over stdin/stdout. It
//! answers discovery and tool listing itself, and forwards tool calls to the
//! running Kiwix app over a same-user channel (see `ipc`). It never opens the
//! library, archives or the profile itself.
//!
//! Client configuration: `{"command": "<Kiwix install folder>/kiwix-mcp", "args": []}`.

mod ipc;
mod protocol;

use serde_json::Value;
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// Longer stdin lines are refused; tool arguments are small.
const MAX_LINE: usize = 1024 * 1024;

struct App {
    /// The profile whose app this relay talks to; `None` is the default profile.
    profile: Option<PathBuf>,
    connection: Option<ipc::Connection>,
}

impl protocol::Backend for App {
    fn call(&mut self, tool: &str, arguments: Value) -> Result<Value, String> {
        // Connect lazily. A connection the app closed (idle timeout, restart,
        // connector toggled) is reopened and the call retried once: every
        // tool is read-only, so a retry is safe.
        for attempt in 0..2 {
            if self.connection.is_none() {
                self.connection = Some(ipc::Connection::open(self.profile.as_deref())?);
            }
            match self.connection.as_mut().unwrap().call(tool, arguments.clone()) {
                Ok(value) => return Ok(value),
                Err(ipc::CallError::App(message)) => return Err(message),
                Err(ipc::CallError::Lost) => {
                    self.connection = None;
                    if attempt == 1 {
                        return Err("Kiwix closed the connection. Try again.".into());
                    }
                }
            }
        }
        unreachable!()
    }
}

fn usage() -> ! {
    println!(
        "kiwix-mcp {} — Kiwix offline-library connector for AI agents (MCP, stdio)\n\n\
         Usage: kiwix-mcp [--data-dir PATH]\n\n\
         Started by an MCP client, not by hand. Kiwix must be running with\n\
         Settings → Agent connector turned on.\n  --data-dir PATH   Only talk to Kiwix running with this profile",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(0)
}

/// Read one line of at most `MAX_LINE` bytes; longer lines are consumed and
/// reported as `Err`. `Ok(None)` at end of input.
fn read_line(input: &mut impl BufRead) -> std::io::Result<Option<Result<String, ()>>> {
    let mut line = Vec::new();
    let mut overflow = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return Ok((!line.is_empty() || overflow)
                .then(|| if overflow { Err(()) } else { Ok(String::from_utf8_lossy(&line).into_owned()) }));
        }
        let (chunk, done) = match buffer.iter().position(|byte| *byte == b'\n') {
            Some(end) => (&buffer[..end], Some(end + 1)),
            None => (buffer, None),
        };
        if !overflow {
            if line.len() + chunk.len() > MAX_LINE {
                overflow = true;
                line.clear();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        let consumed = done.unwrap_or(buffer.len());
        input.consume(consumed);
        if done.is_some() {
            return Ok(Some(if overflow { Err(()) } else { Ok(String::from_utf8_lossy(&line).into_owned()) }));
        }
    }
}

fn main() {
    let mut data_dir = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => match args.next() {
                Some(path) => data_dir = Some(PathBuf::from(path)),
                None => {
                    eprintln!("--data-dir requires a path");
                    std::process::exit(2)
                }
            },
            "--help" | "-h" => usage(),
            "--version" => {
                println!("kiwix-mcp {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!("Unknown option: {other}");
                std::process::exit(2)
            }
        }
    }
    let profile = match ipc::target_profile(data_dir.as_deref()) {
        Ok(profile) => profile,
        Err(error) => {
            eprintln!("--data-dir: {error}");
            std::process::exit(2)
        }
    };
    let mut app = App { profile, connection: None };
    let mut session = protocol::Session::default();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    loop {
        let reply = match read_line(&mut input) {
            Ok(None) | Err(_) => break,
            Ok(Some(Err(()))) => Some(serde_json::json!({"jsonrpc": "2.0", "id": null,
                "error": {"code": -32600, "message": "Request too large"}})),
            Ok(Some(Ok(line))) if line.trim().is_empty() => None,
            Ok(Some(Ok(line))) => session.handle(line.trim_end_matches('\r'), &mut app),
        };
        if let Some(reply) = reply {
            let mut out = stdout.lock();
            // A message is one line: serde_json never emits raw newlines.
            if serde_json::to_writer(&mut out, &reply).is_err() || out.write_all(b"\n").is_err() || out.flush().is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_bounded_and_split() {
        let text = format!("a\n{}\nb", "x".repeat(MAX_LINE + 5));
        let mut input = std::io::BufReader::with_capacity(64, text.as_bytes());
        assert_eq!(read_line(&mut input).unwrap(), Some(Ok("a".into())));
        assert_eq!(read_line(&mut input).unwrap(), Some(Err(())));
        assert_eq!(read_line(&mut input).unwrap(), Some(Ok("b".into())));
        assert_eq!(read_line(&mut input).unwrap(), None);
    }
}
