#![allow(non_snake_case)]

use std::hint;

macro_rules! test {
    ($($name:ident($value:expr, $expected:expr))*) => {
        $(
            #[test]
            fn $name() {
                let mut buffer = itoa::Buffer::new();
                let s = buffer.format(hint::black_box($value));
                assert_eq!(s, $expected);
            }
        )*
    }
}

test! {
    test_u64_0(0u64, "0")
    test_u64_half(u64::from(u32::MAX), "4294967295")
    test_u64_max(u64::MAX, "18446744073709551615")
    test_i64_min(i64::MIN, "-9223372036854775808")

    test_i16_0(0i16, "0")
    test_i16_min(i16::MIN, "-32768")

    test_u128_0(0u128, "0")
    test_u128_max(u128::MAX, "340282366920938463463374607431768211455")
    test_i128_min(i128::MIN, "-170141183460469231731687303715884105728")
    test_i128_max(i128::MAX, "170141183460469231731687303715884105727")
}

#[test]
fn test_max_str_len() {
    use itoa::Integer as _;

    assert_eq!(i8::MAX_STR_LEN, 4);
    assert_eq!(u8::MAX_STR_LEN, 3);
    assert_eq!(i16::MAX_STR_LEN, 6);
    assert_eq!(u16::MAX_STR_LEN, 5);
    assert_eq!(i32::MAX_STR_LEN, 11);
    assert_eq!(u32::MAX_STR_LEN, 10);
    assert_eq!(i64::MAX_STR_LEN, 20);
    assert_eq!(u64::MAX_STR_LEN, 20);
    assert_eq!(i128::MAX_STR_LEN, 40);
    assert_eq!(u128::MAX_STR_LEN, 39);
}

#[test]
fn checked_format_matches_standard_for_every_small_integer() {
    let mut buffer = itoa::Buffer::new();
    for value in u8::MIN..=u8::MAX {
        assert_eq!(buffer.format(value), value.to_string());
        assert_eq!(buffer.format(value as i8), (value as i8).to_string());
    }
    for value in u16::MIN..=u16::MAX {
        assert_eq!(buffer.format(value), value.to_string());
        assert_eq!(buffer.format(value as i16), (value as i16).to_string());
    }
}

#[test]
fn checked_format_matches_standard_across_decimal_widths() {
    let mut buffer = itoa::Buffer::new();
    for n in 0..=38 {
        let power = 10u128.pow(n);
        for value in [power - 1, power, power + 1] {
            assert_eq!(buffer.format(value), value.to_string());
            let signed = value as i128;
            assert_eq!(buffer.format(signed), signed.to_string());
            assert_eq!(buffer.format(-signed), (-signed).to_string());
        }
    }

    let mut state = 0x1234_5678_90ab_cdef_fedc_ba09_8765_4321u128;
    for _ in 0..100_000 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 43;
        macro_rules! check {
            ($($integer:ty),* $(,)?) => {$(
                let value = state as $integer;
                assert_eq!(buffer.format(value), value.to_string());
            )*};
        }
        check!(i32, u32, i64, u64, i128, u128, isize, usize);
    }
}
