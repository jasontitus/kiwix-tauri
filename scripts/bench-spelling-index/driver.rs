// SPDX-License-Identifier: GPL-3.0-or-later
// The collector prepends exact app helpers and both backends to this driver.
use std::{
    fs::File,
    io::{self, BufReader, BufWriter, Read, Write},
    path::Path,
    sync::atomic::AtomicBool,
    time::Instant,
};

const QUERIES: &[&str] = &[
    "climtae",
    "achange",
    "plnaet",
    "kingdm",
    "chnge",
    "xt",
    "aks",
    "wn",
    "qz",
    "suj",
    "sciencee",
    "agencxy",
    "universitx",
    "universitxy",
    "geograhpy",
    "wikipedia",
    "sünd",
    "Reversë",
    "Abuja",
    "Afrcia",
    "ab",
    "abc",
    "stx",
    "Plnaet, EARHT!",
    "climtae change",
    "a plnaet",
    "Cafe\u{301}",
    "STRASSE",
    "nothign fits here",
    "commonprefixaaaaxending",
    "commonprefixzzazending",
    "commonprefxaaaending",
    "commonprefixaxazending",
];

fn titles(input: &mut impl Read) -> io::Result<Vec<String>> {
    let mut titles = Vec::new();
    for _ in 0..256 {
        let mut size = [0; 4];
        if input.read(&mut size[..1])? == 0 {
            break;
        }
        input.read_exact(&mut size[1..])?;
        let length = u32::from_le_bytes(size) as usize;
        if length > 16384 {
            return Err(io::Error::other("title exceeds 16 KiB"));
        }
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes)?;
        titles.push(String::from_utf8(bytes).map_err(io::Error::other)?);
    }
    Ok(titles)
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args[1].as_str();
    let directory = Path::new(&args[2]);
    let file = directory.join("index");
    let flag = AtomicBool::new(false);
    if mode == "extract" {
        let z = zimru::Archive::from_file(File::open(&args[3]).unwrap()).unwrap();
        let ns = if z.header().uses_new_namespaces() {
            b'C'
        } else {
            b'A'
        };
        let range = z.title_prefix_range(ns, "").unwrap();
        let mut out = BufWriter::new(File::create_new(directory).unwrap());
        for i in range {
            let entry = z.title_record(i).unwrap();
            let title = entry.title().as_bytes();
            assert!(title.len() <= 16384);
            out.write_all(&(title.len() as u32).to_le_bytes()).unwrap();
            out.write_all(title).unwrap();
        }
        out.flush().unwrap();
        return;
    }
    let sql = mode.starts_with("sqlite");
    if mode.ends_with("-build") {
        let mut input = BufReader::new(File::open(&args[3]).unwrap());
        let sqlconn = sql.then(|| rusqlite::Connection::open(&file).unwrap());
        let mut conn = sqlconn;
        if let Some(c) = &conn {
            legacy::schema(c).unwrap();
        }
        let scratch = || tempfile::tempfile_in(directory);
        let mut writer = index::Builder::new(&scratch, &flag);
        let (mut total, mut words) = (0, 0);
        loop {
            let batch = titles(&mut input).unwrap();
            if batch.is_empty() {
                break;
            }
            total += batch.len() as u32;
            if let Some(c) = &mut conn {
                words += legacy::add(c, &batch, &flag).unwrap();
            } else {
                for title in &batch {
                    for (start, end) in tokens(title) {
                        let word = normalize(&title[start..end]);
                        if valid_word(&word) {
                            writer.add(word).unwrap();
                        }
                    }
                }
                words = writer.words;
            }
        }
        let mut tracked_peak = None;
        if let Some(c) = conn {
            c.execute(
                "INSERT INTO identity VALUES('benchmark',?1,?2)",
                rusqlite::params![total, words],
            )
            .unwrap();
            let check: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0)).unwrap();
            assert_eq!(check, "ok");
            let observed: u64 = c
                .query_row("SELECT count(*) FROM words", [], |r| r.get(0))
                .unwrap();
            assert_eq!(observed, words);
        } else {
            tracked_peak = Some(
                writer
                    .finish(File::create_new(&file).unwrap(), [0; 32], total)
                    .unwrap(),
            );
            let (_, verified_total, verified_words) =
                index::Lexicon::open(File::open(&file).unwrap(), [0; 32], &flag).unwrap();
            assert_eq!((total, words), (verified_total, verified_words));
        }
        println!(
            "{}",
            serde_json::json!({"titles":total,"words":words,"workingBytes":tracked_peak})
        );
        return;
    }
    assert!(mode.ends_with("-query"));
    let open = Instant::now();
    let conn = sql.then(|| {
        rusqlite::Connection::open_with_flags(&file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap()
    });
    if let Some(c) = &conn {
        c.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA cache_size=-4096; PRAGMA temp_store=MEMORY;").unwrap();
        let integrity: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0)).unwrap();
        assert_eq!(integrity, "ok");
        let _: usize = c
            .query_row("SELECT count(*) FROM words", [], |r| r.get(0))
            .unwrap();
    }
    let checked = (!sql).then(|| {
        index::Lexicon::open(File::open(&file).unwrap(), [0; 32], &flag)
            .unwrap()
            .0
    });
    let open_us = open.elapsed().as_micros();
    let query = Instant::now();
    let (mut first, mut ordered) = (None, Vec::new());
    for pass in 0..32 {
        for q in QUERIES {
            let guesses = if let Some(c) = &conn {
                legacy::query(c, q).unwrap()
            } else {
                corrections_while(checked.as_ref().unwrap(), q, || false).unwrap()
            };
            if pass == 0 {
                ordered.push((
                    q,
                    guesses
                        .into_iter()
                        .map(|g| g.corrected_query)
                        .collect::<Vec<_>>(),
                ));
            }
        }
        if pass == 0 {
            first = Some(query.elapsed().as_micros());
        }
    }
    let first = first.unwrap();
    println!(
        "{}",
        serde_json::json!({"openMicros":open_us,"firstBatchMicros":first,"warmMicros":query.elapsed().as_micros()-first,"warmQueries":31*QUERIES.len(),"results":ordered})
    );
}
