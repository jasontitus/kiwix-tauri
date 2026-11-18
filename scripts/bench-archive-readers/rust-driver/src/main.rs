use std::error::Error;
use std::io::{self, BufRead, Write};
use std::time::Instant;
use zimru::Archive;

fn field(out: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > 65536 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "result field too large",
        ));
    }
    out.write_all(&(bytes.len() as u64).to_le_bytes())?;
    out.write_all(bytes)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("missing mode")?;
    let path = args.next().ok_or("missing archive")?;
    let cache = args.next().ok_or("missing cache policy")?;
    let started = Instant::now();
    let archive = Archive::open(path)?;
    let opened = started.elapsed();
    if cache != "default" {
        archive.set_cluster_cache_max_bytes(cache.parse()?);
    }
    let mut elapsed = opened;
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    if mode == "open" {
        stdout.write_all(&u64::from(archive.all_entry_count()).to_le_bytes())?;
    } else {
        elapsed = std::time::Duration::default();
        let mut scratch = vec![0; 65536];
        let mut warm = mode == "repeat";
        for line in io::stdin().lock().lines() {
            let line = line?;
            let mut fields = line.splitn(3, '\t');
            let offset: u64 = fields.next().ok_or("missing offset")?.parse()?;
            let size: usize = fields.next().ok_or("missing size")?.parse()?;
            let key = fields.next().ok_or("missing key")?;
            let before = Instant::now();
            match mode.as_str() {
                "read" | "repeat" => {
                    let item = archive.get_item(key)?;
                    let n =
                        item.read_at(offset, scratch.get_mut(..size).ok_or("range too large")?)?;
                    if warm {
                        warm = false;
                        continue;
                    }
                    elapsed += before.elapsed();
                    field(&mut stdout, &scratch[..n])?;
                }
                "title" => {
                    let ns = if archive.header().uses_new_namespaces() {
                        b'C'
                    } else {
                        b'A'
                    };
                    let range = archive.title_prefix_range(ns, key)?;
                    let count = u64::from(range.end - range.start);
                    let mut rows = Vec::new();
                    for row in range.take(4) {
                        let entry = archive.entry_by_title_index(row)?;
                        rows.push((entry.path().to_owned(), entry.title().to_owned()));
                    }
                    elapsed += before.elapsed();
                    stdout.write_all(&count.to_le_bytes())?;
                    for (path, title) in rows {
                        field(&mut stdout, path.as_bytes())?;
                        field(&mut stdout, title.as_bytes())?;
                    }
                }
                _ => return Err("unknown mode".into()),
            }
        }
    }
    stdout.flush()?;
    let cache = archive.cluster_cache_stats();
    eprintln!("OPEN_NS {}", opened.as_nanos());
    eprintln!("CACHE {} {}", cache.max_bytes, cache.current_bytes);
    eprintln!("BENCH_NS {}", elapsed.as_nanos());
    Ok(())
}
