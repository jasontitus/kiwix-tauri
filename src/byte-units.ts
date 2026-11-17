// Decimal byte units shared by the formatter. Kept free of React imports so
// the selection logic can be tested directly (Android #5246: 999,999 bytes
// must read 1 MB, never "1000 kB").
export const BYTE_UNITS = [
  "byte",
  "kilobyte",
  "megabyte",
  "gigabyte",
  "terabyte",
  "petabyte",
] as const;

/** Largest unit index whose scaled value stays below 1000 after rounding. */
export function byteUnitIndex(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0;
  let index = Math.min(
    BYTE_UNITS.length - 1,
    Math.floor(Math.log10(value) / 3),
  );
  if (index > 0 && index < BYTE_UNITS.length - 1) {
    // The formatter keeps one decimal above bytes; bump the unit when that
    // rounding would print 1000 of the smaller unit. toFixed mirrors the
    // formatter's half-away-from-zero rounding at this precision.
    const rounded = Number((value / 1000 ** index).toFixed(1));
    if (rounded >= 1000) index += 1;
  }
  return index;
}
