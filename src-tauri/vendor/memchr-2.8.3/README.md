# Checked memchr for Kiwix

This private fork supplies the slice-based byte and substring APIs used by
Kiwix and the MCP relay. It uses checked word loads and the portable Two-Way
searcher. The crate forbids unsafe Rust. The supported surface is the crate-root byte searches and `memmem`. The
portable helpers used by those APIs remain available. Other low-level
`arch` APIs from the published crate are omitted.

See SOURCE.txt for origin, changes, and license information.
