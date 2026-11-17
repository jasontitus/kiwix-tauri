// Match publisher.rs before admitting a durable preference write. An invalid
// draft must never become a failed settings-queue entry that blocks resume.
const encoder = new TextEncoder();
const prohibited = /[\p{Cc}\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069\ud800-\udfff]/u;

export function validSharingName(value: string): boolean {
  return value.trim() === value && !prohibited.test(value)
    && encoder.encode(value).byteLength <= 160;
}
