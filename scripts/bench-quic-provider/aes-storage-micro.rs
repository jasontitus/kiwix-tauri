#![forbid(unsafe_code)]

use aes::{
    Aes128, Aes256, Block,
    cipher::{BlockCipherEncrypt, Key, KeyInit},
};
use std::{hint::black_box, time::{Duration, Instant}};

const KEYS: usize = 40_000;
const BLOCKS: usize = 1_000_000;

fn time(mut op: impl FnMut(), count: usize) -> Duration {
    let start = Instant::now();
    for _ in 0..count {
        op();
    }
    start.elapsed()
}

fn bench<C: KeyInit + BlockCipherEncrypt<BlockSize = aes::cipher::consts::U16>>(suite: &str) {
    let mut key = Key::<C>::default();
    key.fill(0x42);
    let mut block = Block::from([0x1f; 16]);
    let cipher = C::new(&key);

    let hot = time(|| {
        cipher.encrypt_block(black_box(&mut block));
        black_box(&block);
    }, BLOCKS);

    let with_key = time(|| {
        let cipher = C::new(black_box(&key));
        cipher.encrypt_block(black_box(&mut block));
        black_box(&block);
    }, BLOCKS);

    let keys: Vec<_> = (0..KEYS).map(|_| C::new(black_box(&key))).collect();
    let start = Instant::now();
    drop(black_box(keys));
    let erase = start.elapsed();

    for (operation, duration, count) in [
        ("encrypt", hot, BLOCKS),
        ("init_encrypt_drop", with_key, BLOCKS),
        ("drop", erase, KEYS),
    ] {
        println!("{suite} {operation} {:.3}", duration.as_secs_f64() * 1e9 / count as f64);
    }
    println!("{suite} state_bytes {}", std::mem::size_of::<C>());
}

fn main() {
    println!("aes_hardware {}", aes::hardware_accelerated());
    bench::<Aes128>("AES128");
    bench::<Aes256>("AES256");
}
