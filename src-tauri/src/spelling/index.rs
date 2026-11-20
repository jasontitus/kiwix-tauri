// SPDX-License-Identifier: GPL-3.0-or-later
//! Immutable forward/reverse lexicons. The reader retains one anchor and digest
//! per data page. Each query rechecks only the pages it actually reads.
#![forbid(unsafe_code)]
use super::{
    check_cancel, distance_one, valid_word, MAX_CANDIDATES, MAX_DB_BYTES, MAX_TITLES, MAX_WORDS,
};
use chksum_hash_sha2_256 as sha256;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

const PAGE: usize = 8192;
const HEAD: u64 = 64;
const MAGIC: &[u8; 8] = b"KIWISPL2";
const CHUNK: usize = 2 * 1024 * 1024;
const BLOOM_BITS: usize = 1 << 24;
type Result<T> = io::Result<T>;
type Key = (u8, String);
type Entry = (Key, u64);

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Damaged or oversized spelling index",
    )
}

fn cancelled(flag: &AtomicBool) -> Result<()> {
    check_cancel(flag).map_err(io::Error::other)
}

fn key(text: String) -> Key {
    (text.chars().count() as u8, text)
}

fn read_at(file: &File, mut offset: u64, mut bytes: &mut [u8]) -> Result<()> {
    while !bytes.is_empty() {
        #[cfg(unix)]
        let read = std::os::unix::fs::FileExt::read_at(file, bytes, offset);
        #[cfg(windows)]
        let read = std::os::windows::fs::FileExt::seek_read(file, bytes, offset);
        match read {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                bytes = &mut bytes[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn visit_page(
    page: &[u8; PAGE],
    mut visit: impl FnMut(u8, &str, u64) -> Result<bool>,
) -> Result<()> {
    let used = usize::from(u16::from_le_bytes([page[0], page[1]]));
    let mut bytes = page.get(2..used).ok_or_else(invalid)?;
    if bytes.is_empty() || page[used..].iter().any(|b| *b != 0) {
        return Err(invalid());
    }
    while !bytes.is_empty() {
        let header = bytes.get(..11).ok_or_else(invalid)?;
        let len = usize::from(u16::from_le_bytes([header[0], header[1]]));
        let count = u64::from_le_bytes(header[3..11].try_into().unwrap());
        if !(2..=256).contains(&len) || count == 0 {
            return Err(invalid());
        }
        let word = std::str::from_utf8(bytes.get(11..11 + len).ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        if !visit(header[2], word, count)? {
            break;
        }
        bytes = &bytes[11 + len..];
    }
    Ok(())
}

// Total logical bytes held by all live merge runs and the new publication.
// The old read-only dictionary, if present, remains independently bounded.
const MAX_BUILD_BYTES: u64 = 2 * MAX_DB_BYTES;
#[derive(Default)]
struct Budget {
    live: AtomicU64,
    peak: AtomicU64,
}

struct Reservation {
    budget: Arc<Budget>,
    bytes: u64,
}
impl Reservation {
    fn new(budget: &Arc<Budget>) -> Self {
        Self {
            budget: budget.clone(),
            bytes: 0,
        }
    }
    fn grow(&mut self, bytes: u64) -> Result<()> {
        let previous = self.budget.live.fetch_add(bytes, Ordering::Relaxed);
        if previous
            .checked_add(bytes)
            .is_none_or(|sum| sum > MAX_BUILD_BYTES)
        {
            self.budget.live.fetch_sub(bytes, Ordering::Relaxed);
            return Err(io::Error::other(
                "Spelling build exceeds 2 GiB of working storage",
            ));
        }
        self.budget
            .peak
            .fetch_max(previous + bytes, Ordering::Relaxed);
        self.bytes += bytes;
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.live.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
type CreateScratch<'a> = &'a dyn Fn() -> Result<File>;

struct Anchor {
    first: Key,
    digest: [u8; 32],
}

// One immutable allocation for the page and one compact offset per word. Binary
// search then examines only the touched UTF-8 keys, even for tiny words.
struct Block {
    raw: Box<[u8; PAGE]>,
    offsets: Box<[u16]>,
}

impl Block {
    fn parse(raw: Box<[u8; PAGE]>) -> Result<Self> {
        let mut offset = 2;
        let mut offsets = Vec::new();
        visit_page(&raw, |_, word, _| {
            offsets.push(offset as u16);
            offset += 11 + word.len();
            Ok(true)
        })?;
        Ok(Self {
            raw,
            offsets: offsets.into_boxed_slice(),
        })
    }

    fn row(&self, offset: u16) -> (u8, &str, u64) {
        let i = usize::from(offset);
        let len = u16::from_le_bytes([self.raw[i], self.raw[i + 1]]) as usize;
        let count = u64::from_le_bytes(self.raw[i + 3..i + 11].try_into().unwrap());
        // The bytes and offsets are immutable after the checked page parse.
        (
            self.raw[i + 2],
            std::str::from_utf8(&self.raw[i + 11..i + 11 + len]).unwrap(),
            count,
        )
    }
}

struct Table {
    _reservation: Option<Reservation>,
    cached: RefCell<VecDeque<(usize, Arc<Block>)>>,
    file: Arc<File>,
    base: u64,
    anchors: Vec<Anchor>,
}

impl Table {
    fn block(&self, index: usize) -> Result<Arc<Block>> {
        let mut cached = self.cached.borrow_mut();
        if let Some(i) = cached.iter().position(|(n, _)| *n == index) {
            let pair = cached.remove(i).unwrap();
            let block = pair.1.clone();
            cached.push_front(pair);
            return Ok(block);
        }
        let mut page = Box::new([0; PAGE]);
        read_at(
            &self.file,
            self.base + index as u64 * PAGE as u64,
            &mut page[..],
        )?;
        if <[u8; 32]>::from(sha256::hash(&page[..])) != self.anchors[index].digest {
            return Err(invalid());
        }
        let block = Arc::new(Block::parse(page)?);
        if cached.len() == 64 {
            cached.pop_back();
        }
        cached.push_front((index, block.clone()));
        Ok(block)
    }

    fn page_estimate(&self, length: u8, prefix: &str) -> usize {
        let start = self
            .anchors
            .partition_point(|a| (a.first.0, a.first.1.as_str()) <= (length, prefix))
            .saturating_sub(1);
        let end = self.anchors.partition_point(|a| {
            a.first.0 < length
                || (a.first.0 == length
                    && (a.first.1.as_str() < prefix || a.first.1.starts_with(prefix)))
        });
        end.saturating_sub(start)
    }

    fn scan(
        &self,
        length: u8,
        prefix: &str,
        exact: bool,
        mut visit: impl FnMut(&str, u64) -> Result<()>,
    ) -> Result<()> {
        let first = self
            .anchors
            .partition_point(|a| (a.first.0, a.first.1.as_str()) <= (length, prefix))
            .saturating_sub(1);
        for n in first..self.anchors.len() {
            let anchor = &self.anchors[n];
            if n != first && (anchor.first.0 != length || !anchor.first.1.starts_with(prefix)) {
                break;
            }
            let block = self.block(n)?;
            let begin = if n == first {
                block.offsets.partition_point(|i| {
                    let (width, word, _) = block.row(*i);
                    (width, word) < (length, prefix)
                })
            } else {
                0
            };
            for i in &block.offsets[begin..] {
                let (width, word, freq) = block.row(*i);
                if width != length || !word.starts_with(prefix) || (exact && word != prefix) {
                    return Ok(());
                }
                visit(word, freq)?;
                if exact {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn get(&self, k: &Key) -> Result<Option<u64>> {
        let mut result = None;
        self.scan(k.0, &k.1, true, |_, count| {
            result = Some(count);
            Ok(())
        })?;
        Ok(result)
    }

    fn iter(&self) -> Rows<'_> {
        Rows {
            table: self,
            page: 0,
            rows: Vec::new().into_iter(),
        }
    }
}

struct Rows<'a> {
    table: &'a Table,
    page: usize,
    rows: std::vec::IntoIter<Entry>,
}
impl Rows<'_> {
    fn next(&mut self) -> Result<Option<Entry>> {
        if let Some(row) = self.rows.next() {
            return Ok(Some(row));
        }
        let Some(anchor) = self.table.anchors.get(self.page) else {
            return Ok(None);
        };
        let mut bytes = [0; PAGE];
        read_at(
            &self.table.file,
            self.table.base + self.page as u64 * PAGE as u64,
            &mut bytes,
        )?;
        if <[u8; 32]>::from(sha256::hash(bytes)) != anchor.digest {
            return Err(invalid());
        }
        let mut list = Vec::new();
        visit_page(&bytes, |length, word, n| {
            list.push(((length, word.to_owned()), n));
            Ok(true)
        })?;
        self.page += 1;
        self.rows = list.into_iter();
        Ok(self.rows.next())
    }
}

struct Pages<'a> {
    reservation: Reservation,
    out: BufWriter<&'a mut File>,
    page: [u8; PAGE],
    used: usize,
    first: Option<Key>,
    anchors: Vec<Anchor>,
}

impl<'a> Pages<'a> {
    fn new(out: &'a mut File, budget: &Arc<Budget>) -> Self {
        Self {
            reservation: Reservation::new(budget),
            out: BufWriter::new(out),
            page: [0; PAGE],
            used: 2,
            first: None,
            anchors: Vec::new(),
        }
    }
    fn add(&mut self, k: &Key, n: u64) -> Result<()> {
        let word = k.1.as_bytes();
        if word.len() > 256 || n == 0 {
            return Err(invalid());
        }
        if self.used + 11 + word.len() > PAGE {
            self.flush_page()?;
        }
        if self.first.is_none() {
            self.first = Some(k.clone());
        }
        let i = self.used;
        self.page[i..i + 2].copy_from_slice(&(word.len() as u16).to_le_bytes());
        self.page[i + 2] = k.0;
        self.page[i + 3..i + 11].copy_from_slice(&n.to_le_bytes());
        self.page[i + 11..i + 11 + word.len()].copy_from_slice(word);
        self.used += 11 + word.len();
        Ok(())
    }
    fn flush_page(&mut self) -> Result<()> {
        let Some(first) = self.first.take() else {
            return Ok(());
        };
        if self.anchors.len() as u64 >= MAX_DB_BYTES / PAGE as u64 {
            return Err(invalid());
        }
        self.page[..2].copy_from_slice(&(self.used as u16).to_le_bytes());
        self.reservation.grow(PAGE as u64)?;
        self.out.write_all(&self.page)?;
        self.anchors.push(Anchor {
            first,
            digest: sha256::hash(self.page).into(),
        });
        self.page.fill(0);
        self.used = 2;
        Ok(())
    }
    fn finish(mut self) -> Result<(Vec<Anchor>, Reservation)> {
        self.flush_page()?;
        self.out.flush()?;
        Ok((self.anchors, self.reservation))
    }
}

// Spill sorted runs and combine equally sized generations. A single archive can
// contribute arbitrarily many copies of a word without retaining those copies.
struct Sorter<'a> {
    create: CreateScratch<'a>,
    budget: Arc<Budget>,
    chunk: BTreeMap<Key, u64>,
    bytes: usize,
    levels: Vec<Option<Table>>,
    flag: &'a AtomicBool,
}

impl<'a> Sorter<'a> {
    fn new(create: CreateScratch<'a>, flag: &'a AtomicBool, budget: Arc<Budget>) -> Self {
        Self {
            create,
            budget,
            chunk: BTreeMap::new(),
            bytes: 0,
            levels: Vec::new(),
            flag,
        }
    }
    fn add(&mut self, k: Key, n: u64) -> Result<()> {
        if let Some(old) = self.chunk.get_mut(&k) {
            *old = old.checked_add(n).ok_or_else(invalid)?;
        } else {
            self.bytes += 96 + k.1.len();
            self.chunk.insert(k, n);
        }
        if self.bytes >= CHUNK {
            self.spill()?;
        }
        Ok(())
    }
    fn spill(&mut self) -> Result<()> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        let mut file = (self.create)()?;
        let mut pages = Pages::new(&mut file, &self.budget);
        for (i, (k, n)) in std::mem::take(&mut self.chunk).into_iter().enumerate() {
            if i % 256 == 0 {
                cancelled(self.flag)?;
            }
            pages.add(&k, n)?;
        }
        let (anchors, reservation) = pages.finish()?;
        let mut run = Table {
            _reservation: Some(reservation),
            cached: Default::default(),
            file: Arc::new(file),
            base: 0,
            anchors,
        };
        self.bytes = 0;
        for level in &mut self.levels {
            if let Some(other) = level.take() {
                run = merge(self.create, &self.budget, other, run, self.flag)?;
            } else {
                *level = Some(run);
                return Ok(());
            }
        }
        self.levels.push(Some(run));
        Ok(())
    }
    fn finish(mut self) -> Result<Table> {
        self.spill()?;
        let mut runs = self.levels.into_iter().flatten();
        let Some(mut run) = runs.next() else {
            return Ok(Table {
                _reservation: None,
                cached: Default::default(),
                file: Arc::new((self.create)()?),
                base: 0,
                anchors: Vec::new(),
            });
        };
        for other in runs {
            run = merge(self.create, &self.budget, run, other, self.flag)?;
        }
        Ok(run)
    }
}

fn merge(
    create: CreateScratch<'_>,
    budget: &Arc<Budget>,
    a: Table,
    b: Table,
    flag: &AtomicBool,
) -> Result<Table> {
    let mut file = create()?;
    let mut pages = Pages::new(&mut file, budget);
    let (mut a, mut b) = (a.iter(), b.iter());
    let (mut left, mut right) = (a.next()?, b.next()?);
    let mut rows = 0u32;
    while left.is_some() || right.is_some() {
        if rows.is_multiple_of(256) {
            cancelled(flag)?;
        }
        rows += 1;
        if rows as u64 > MAX_WORDS {
            return Err(invalid());
        }
        let order = match (&left, &right) {
            (Some(x), Some(y)) => x.0.cmp(&y.0),
            (Some(_), None) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Greater,
        };
        match order {
            std::cmp::Ordering::Less => {
                let (k, n) = left.take().unwrap();
                pages.add(&k, n)?;
                left = a.next()?;
            }
            std::cmp::Ordering::Greater => {
                let (k, n) = right.take().unwrap();
                pages.add(&k, n)?;
                right = b.next()?;
            }
            std::cmp::Ordering::Equal => {
                let (k, x) = left.take().unwrap();
                let (_, y) = right.take().unwrap();
                pages.add(&k, x.checked_add(y).ok_or_else(invalid)?)?;
                left = a.next()?;
                right = b.next()?;
            }
        }
    }
    let (anchors, reservation) = pages.finish()?;
    Ok(Table {
        _reservation: Some(reservation),
        cached: Default::default(),
        file: Arc::new(file),
        anchors,
        base: 0,
    })
}

pub(super) struct Builder<'a> {
    forward: Sorter<'a>,
    bloom: Vec<u64>,
    hash: std::hash::RandomState,
    pub words: u64,
}

impl<'a> Builder<'a> {
    pub fn new(create: CreateScratch<'a>, flag: &'a AtomicBool) -> Self {
        Self {
            forward: Sorter::new(create, flag, Arc::default()),
            bloom: Vec::new(),
            hash: Default::default(),
            words: 0,
        }
    }

    pub fn add(&mut self, word: String) -> Result<()> {
        use std::hash::BuildHasher;
        let k = key(word);
        if self.forward.chunk.contains_key(&k) {
            return self.forward.add(k, 1);
        }
        if self.bloom.is_empty() {
            self.bloom.resize(BLOOM_BITS / 64, 0);
        }
        let h = self.hash.hash_one(&k);
        let indices = [h as usize % BLOOM_BITS, (h >> 32) as usize % BLOOM_BITS];
        let possible = indices
            .iter()
            .all(|i| self.bloom[i / 64] & (1 << (i % 64)) != 0);
        let mut seen = false;
        if possible {
            for run in self.forward.levels.iter().flatten() {
                if run.get(&k)?.is_some() {
                    seen = true;
                    break;
                }
            }
        }
        if !seen {
            self.words += 1;
            if self.words > MAX_WORDS {
                return Err(invalid());
            }
        }
        for i in indices {
            self.bloom[i / 64] |= 1 << (i % 64);
        }
        self.forward.add(k, 1)
    }

    pub fn finish(self, mut out: File, identity: [u8; 32], total: u32) -> Result<u64> {
        let create = self.forward.create;
        let budget = self.forward.budget.clone();
        let flag = self.forward.flag;
        let forward = self.forward.finish()?;
        let mut reverse = Sorter::new(create, flag, budget.clone());
        let mut rows = forward.iter();
        while let Some(((length, word), n)) = rows.next()? {
            cancelled(flag)?;
            reverse.add((length, word.chars().rev().collect()), n)?;
        }
        drop(rows);
        let reverse = reverse.finish()?;
        let counts = [forward.anchors.len() as u32, reverse.anchors.len() as u32];
        let size = HEAD + u64::from(counts[0] + counts[1]) * PAGE as u64 + 32;
        if size > MAX_DB_BYTES {
            return Err(invalid());
        }
        let mut header = [0; HEAD as usize];
        header[..8].copy_from_slice(MAGIC);
        header[8..40].copy_from_slice(&identity);
        header[40..44].copy_from_slice(&total.to_le_bytes());
        header[44..48].copy_from_slice(&(self.words as u32).to_le_bytes());
        header[48..52].copy_from_slice(&counts[0].to_le_bytes());
        header[52..56].copy_from_slice(&counts[1].to_le_bytes());
        out.seek(SeekFrom::Start(0))?;
        let mut writer = BufWriter::new(&mut out);
        let mut final_bytes = Reservation::new(&budget);
        final_bytes.grow(HEAD)?;
        writer.write_all(&header)?;
        let mut integrity = sha256::default();
        integrity.update(header);
        for table in [forward, reverse] {
            let mut buf = [0; PAGE];
            for (i, anchor) in table.anchors.iter().enumerate() {
                cancelled(flag)?;
                read_at(&table.file, i as u64 * PAGE as u64, &mut buf)?;
                let digest: [u8; 32] = sha256::hash(buf).into();
                if digest != anchor.digest {
                    return Err(invalid());
                }
                integrity.update(digest);
                final_bytes.grow(PAGE as u64)?;
                writer.write_all(&buf)?;
            }
        }
        final_bytes.grow(32)?;
        writer.write_all(integrity.digest().as_ref())?;
        writer.flush()?;
        drop(writer);
        out.sync_all()?;
        Ok(budget.peak.load(Ordering::Relaxed))
    }
}

pub(super) struct Lexicon {
    forward: Table,
    reverse: Table,
}

impl Lexicon {
    pub fn open(file: File, id: [u8; 32], flag: &AtomicBool) -> Result<(Self, u32, u64)> {
        let meta = file.metadata()?;
        if !meta.is_file() || meta.len() > MAX_DB_BYTES {
            return Err(invalid());
        }
        let mut reader = BufReader::new(&file);
        let mut header = [0; HEAD as usize];
        reader.read_exact(&mut header)?;
        let number =
            |start: usize| u32::from_le_bytes(header[start..start + 4].try_into().unwrap());
        let (total, words) = (number(40), number(44));
        let counts = [number(48), number(52)];
        if &header[..8] != MAGIC
            || header[8..40] != id
            || header[56..] != [0; 8]
            || total > MAX_TITLES
            || u64::from(words) > MAX_WORDS
            || HEAD
                + counts
                    .iter()
                    .map(|c| u64::from(*c) * PAGE as u64)
                    .sum::<u64>()
                + 32
                != meta.len()
        {
            return Err(invalid());
        }
        let mut integrity = sha256::default();
        integrity.update(header);
        // A fresh keyed multiset fingerprint also checks that both independently
        // ordered tables describe the same words and frequencies. The file never
        // controls the fingerprint keys.
        use std::hash::BuildHasher;
        let hashers = [std::hash::RandomState::new(), std::hash::RandomState::new()];
        let mut fingerprints = [[0u64; 2]; 2];
        let mut lists = [Vec::new(), Vec::new()];
        for (section, anchors) in lists.iter_mut().enumerate() {
            let mut previous = None;
            let mut entries = 0;
            let mut remaining = 0;
            for i in 0..counts[section] {
                cancelled(flag)?;
                let mut page = [0; PAGE];
                reader.read_exact(&mut page)?;
                let digest: [u8; 32] = sha256::hash(page).into();
                integrity.update(digest);
                let mut first = true;
                visit_page(&page, |width, word, count| {
                    let k = (width, word.to_owned());
                    if !valid_word(word)
                        || width as usize != word.chars().count()
                        || count > u64::from(total) * 8192
                        || previous.as_ref().is_some_and(|p| p >= &k)
                    {
                        return Err(invalid());
                    }
                    let original;
                    let canonical = if section == 0 {
                        word
                    } else {
                        original = word.chars().rev().collect::<String>();
                        &original
                    };
                    for (fingerprint, hasher) in fingerprints[section].iter_mut().zip(&hashers) {
                        *fingerprint =
                            fingerprint.wrapping_add(hasher.hash_one((canonical, count)));
                    }
                    if first {
                        if i > 0 && remaining >= 11 + word.len() {
                            return Err(invalid());
                        }
                        anchors.push(Anchor {
                            first: k.clone(),
                            digest,
                        });
                        first = false;
                    }
                    previous = Some(k);
                    entries += 1;
                    if entries > words {
                        return Err(invalid());
                    }
                    Ok(true)
                })?;
                remaining = PAGE - usize::from(u16::from_le_bytes([page[0], page[1]]));
            }
            if entries != words {
                return Err(invalid());
            }
        }
        let mut stored = [0; 32];
        reader.read_exact(&mut stored)?;
        if stored != <[u8; 32]>::from(integrity.digest()) || fingerprints[0] != fingerprints[1] {
            return Err(invalid());
        }
        drop(reader);
        let file = Arc::new(file);
        let [a, b] = lists;
        Ok((
            Self {
                forward: Table {
                    _reservation: None,
                    cached: Default::default(),
                    anchors: a,
                    file: file.clone(),
                    base: HEAD,
                },
                reverse: Table {
                    _reservation: None,
                    cached: Default::default(),
                    anchors: b,
                    file,
                    base: HEAD + u64::from(counts[0]) * PAGE as u64,
                },
            },
            total,
            u64::from(words),
        ))
    }
    pub fn known(&self, word: &str) -> Result<bool> {
        Ok(self.forward.get(&key(word.to_owned()))?.is_some())
    }

    pub fn candidates(
        &self,
        word: &str,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<Vec<(String, u64)>> {
        let poll = || {
            if is_cancelled() {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Search cancelled",
                ))
            } else {
                Ok(())
            }
        };
        poll()?;
        let mut examined = 0usize;
        let chars = word.chars().collect::<Vec<_>>();
        let n = chars.len();
        if !(2..=64).contains(&n) {
            return Err(invalid());
        }
        let halves = |m: usize| {
            (
                chars[..m].iter().collect::<String>(),
                chars[m..].iter().rev().collect::<String>(),
            )
        };
        let cost = |prefix: &str, suffix: &str| {
            ((n - 1).max(2)..=(n + 1).min(64))
                .map(|length| {
                    self.forward.page_estimate(length as u8, prefix)
                        + self.reverse.page_estimate(length as u8, suffix)
                })
                .sum::<usize>()
        };
        let mut m = n / 2;
        let (mut prefix, mut suffix) = halves(m);
        let mut pages = cost(&prefix, &suffix);
        // A common prefix can otherwise select most of a length partition.
        // Estimate alternatives from the sparse in-memory anchors only.
        if pages > 12 {
            for split in 1..n {
                if split == m {
                    continue;
                }
                let (p, s) = halves(split);
                let candidate = cost(&p, &s);
                if candidate < pages {
                    (m, prefix, suffix, pages) = (split, p, s, candidate);
                }
                if pages <= 6 {
                    break;
                }
            }
        }
        let mut seen = HashSet::new();
        let mut best = Vec::new();
        let mut accept = |candidate: String, freq: u64| {
            if seen.insert(candidate.clone()) {
                if seen.len() > MAX_CANDIDATES {
                    return Err(io::Error::other("Spelling candidate limit exceeded"));
                }
                best.push((candidate, freq));
                best.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                best.truncate(5);
            }
            Ok(())
        };
        let reversed: String = chars.iter().rev().collect();
        for length in (n - 1).max(2)..=(n + 1).min(64) {
            self.forward.scan(length as u8, &prefix, false, |s, f| {
                if examined.is_multiple_of(64) {
                    poll()?;
                }
                examined += 1;
                if distance_one(word, s) {
                    accept(s.to_owned(), f)?;
                }
                Ok(())
            })?;
            self.reverse.scan(length as u8, &suffix, false, |s, f| {
                if examined.is_multiple_of(64) {
                    poll()?;
                }
                examined += 1;
                if distance_one(&reversed, s) {
                    accept(s.chars().rev().collect(), f)?;
                }
                Ok(())
            })?;
        }
        // Every one-edit neighbor preserves a complete half of the query except
        // a transposition across its split. Check that one possible word directly.
        let mut swapped = chars;
        swapped.swap(m - 1, m);
        let cross = (n as u8, swapped.iter().collect());
        if cross.1 != word {
            if let Some(f) = self.forward.get(&cross)? {
                accept(cross.1, f)?;
            }
        }
        Ok(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_words() -> Vec<(String, u64)> {
        // Every string of length 2..4 over a mixed-width alphabet; gives all
        // four edits on either side and directly across every midpoint.
        let mut layer = vec![String::new()];
        let mut lexicon = Vec::new();
        for width in 1..=4 {
            layer = layer
                .iter()
                .flat_map(|base| {
                    ['a', 'b', 'é', '猫']
                        .into_iter()
                        .map(move |ch| format!("{base}{ch}"))
                })
                .collect();
            if width >= 2 {
                for (i, word) in layer.iter().enumerate() {
                    lexicon.push((word.clone(), (i % 7 + 1) as u64));
                }
            }
        }
        lexicon
    }

    #[test]
    fn all_edit_operations_rank_like_exhaustive_search_and_check_integrity() {
        let dir = super::super::tests::TestDirectory::new();
        let directory = super::super::open_directory(&dir.0, true).unwrap();
        let create = || super::super::scratch_file(&directory);
        let path = dir.0.join("dictionary");
        let cancel = AtomicBool::new(false);
        let mut writer = Builder::new(&create, &cancel);
        let words = in_memory_words();
        for (word, frequency) in &words {
            for _ in 0..*frequency {
                writer.add(word.clone()).unwrap();
            }
        }
        let peak = writer
            .finish(File::create_new(&path).unwrap(), [23; 32], 100)
            .unwrap();
        assert!(peak > path.metadata().unwrap().len());
        let (lexicon, _, count) =
            Lexicon::open(File::open(&path).unwrap(), [23; 32], &cancel).unwrap();
        assert_eq!(count, words.len() as u64);
        for (query, _) in &words {
            let mut expected: Vec<_> = words
                .iter()
                .filter(|(w, _)| distance_one(query, w))
                .cloned()
                .collect();
            expected.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            expected.truncate(5);
            assert_eq!(
                lexicon.candidates(query, || false).unwrap(),
                expected,
                "{query}"
            );
        }
        assert!(lexicon.candidates("ba", || true).is_err());
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(Lexicon::open(File::open(&path).unwrap(), [24; 32], &cancel).is_err());
        bytes[HEAD as usize + 13] ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        assert!(Lexicon::open(File::open(&path).unwrap(), [23; 32], &cancel).is_err());
    }

    #[test]
    fn broad_supersets_preserve_ranking_and_cap_only_live_neighbors() {
        let dir = super::super::tests::TestDirectory::new();
        let directory = super::super::open_directory(&dir.0, true).unwrap();
        let create = || super::super::scratch_file(&directory);
        let flag = AtomicBool::new(false);
        let mut builder = Builder::new(&create, &flag);
        for i in 0..16_000_u32 {
            let word = format!(
                "abc{}{}{}",
                char::from_u32(97 + i / 676).unwrap(),
                char::from_u32(97 + i / 26 % 26).unwrap(),
                char::from_u32(97 + i % 26).unwrap()
            );
            builder.add(word).unwrap();
            if i < 8_200 {
                builder
                    .add(format!("a{}b", char::from_u32(0x4e00 + i).unwrap()))
                    .unwrap();
            }
        }
        // Repeated after spilling across several generations.
        for _ in 0..12 {
            builder.add("abcdeg".to_owned()).unwrap();
        }
        let path = dir.0.join("broad");
        builder
            .finish(File::create_new(&path).unwrap(), [0; 32], 25_000)
            .unwrap();
        let (lex, _, _) = Lexicon::open(File::open(&path).unwrap(), [0; 32], &flag).unwrap();
        assert_eq!(
            lex.candidates("abcdef", || false).unwrap()[0],
            ("abcdeg".into(), 13)
        );
        assert!(lex
            .candidates("a猫b", || false)
            .unwrap_err()
            .to_string()
            .contains("candidate limit"));
        let calls = std::cell::Cell::new(0);
        let reason = lex
            .candidates("a猫b", || {
                calls.set(calls.get() + 1);
                calls.get() == 3
            })
            .unwrap_err();
        assert_eq!(reason.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn page_substitution_and_inconsistent_reverse_frequency_are_rejected() {
        let dir = super::super::tests::TestDirectory::new();
        let directory = super::super::open_directory(&dir.0, true).unwrap();
        let create = || super::super::scratch_file(&directory);
        let flag = AtomicBool::new(false);
        let mut builder = Builder::new(&create, &flag);
        builder.add("planet".into()).unwrap();
        let path = dir.0.join("checked");
        builder
            .finish(File::create_new(&path).unwrap(), [7; 32], 1)
            .unwrap();
        let (lex, _, _) = Lexicon::open(File::open(&path).unwrap(), [7; 32], &flag).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let original = bytes.clone();
        bytes[HEAD as usize + 5] = 2;
        std::fs::write(&path, &bytes).unwrap();
        assert!(lex.known("planet").is_err());
        bytes = original;
        bytes[HEAD as usize + PAGE + 5] = 2;
        let mut crc = sha256::default();
        crc.update(&bytes[..HEAD as usize]);
        for page in bytes[HEAD as usize..HEAD as usize + 2 * PAGE].chunks_exact(PAGE) {
            crc.update(sha256::hash(page));
        }
        bytes[HEAD as usize + 2 * PAGE..].copy_from_slice(crc.digest().as_ref());
        std::fs::write(&path, bytes).unwrap();
        assert!(Lexicon::open(File::open(&path).unwrap(), [7; 32], &flag).is_err());
    }

    #[test]
    fn working_storage_budget_releases_failed_merges() {
        let budget = Arc::<Budget>::default();
        let mut first = Reservation::new(&budget);
        let mut second = Reservation::new(&budget);
        first.grow(MAX_BUILD_BYTES - 1).unwrap();
        assert!(second.grow(2).is_err());
        assert_eq!(second.bytes, 0);
        second.grow(1).unwrap();
        drop(first);
        second.grow(17).unwrap();
        assert_eq!(budget.peak.load(Ordering::Relaxed), MAX_BUILD_BYTES);
        assert_eq!(budget.live.load(Ordering::Relaxed), 18);
        drop(second);
        assert_eq!(budget.live.load(Ordering::Relaxed), 0);
    }
}
