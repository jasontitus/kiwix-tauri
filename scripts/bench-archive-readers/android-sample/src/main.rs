#![forbid(unsafe_code)]

use nix::sys::resource::{getrusage, UsageWho};
use nix::sys::time::TimeValLike;
use std::{env, error::Error, process::Command, time::Instant};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let program = args.next().ok_or("missing reader executable")?;
    let started = Instant::now();
    let status = Command::new(program).args(args).status()?;
    let wall = started.elapsed().as_nanos();
    if !status.success() {
        return Err(format!("reader exited with {status}").into());
    }

    // This wrapper launched and waited for exactly one reader. On Android the
    // kernel reports the child's peak resident size in KiB.
    let usage = getrusage(UsageWho::RUSAGE_CHILDREN)?;
    let cpu_us = usage.user_time().num_microseconds() + usage.system_time().num_microseconds();
    eprintln!("PROCESS {wall} {cpu_us} {}", usage.max_rss());
    Ok(())
}
