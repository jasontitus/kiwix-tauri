# Audit evidence

The reviewed audit reports remain as Markdown. The raw issue/PR census JSON is stored as lossless XZ-compressed files:

- `*.raw.json.xz`

Decompress with `xz -dk file.raw.json.xz` (or `xz -dc file.raw.json.xz | jq ...`). The compression round-trip was verified by SHA-256 before the uncompressed files were removed. SHA-256 values printed in the reports refer to the decompressed JSON bytes.
