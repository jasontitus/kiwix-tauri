// SPDX-License-Identifier: GPL-3.0-or-later
//! Print the app's Unicode full default case-fold mappings from its locked ICU data.
use std::collections::BTreeMap;

fn main() {
    let folder = icu_casemap::CaseMapper::new();
    let mut pages = BTreeMap::<u32, Vec<(u8, String)>>::new();
    for value in (128..=0x10ffff).filter_map(char::from_u32) {
        let mut utf8 = [0; 4];
        let original = value.encode_utf8(&mut utf8);
        let folded = folder.fold_string(original);
        if folded != original as &str {
            pages
                .entry(u32::from(value) >> 8)
                .or_default()
                .push((u32::from(value) as u8, folded.into_owned()));
        }
    }
    println!("// SPDX-License-Identifier: Unicode-3.0");
    println!("// Generated with generate_casefold from ICU4X 2.4.0 (Unicode 18).");
    println!("// See src-tauri/licenses/glass/Unicode-3.0.txt.");
    println!("pub(super) fn page(high: u32) -> &'static [(u8, &'static str)] {{");
    println!("    match high {{");
    for (high, entries) in pages {
        println!("        {high:#x} => &[");
        for (low, folded) in entries {
            let escaped: String = folded.chars().flat_map(char::escape_unicode).collect();
            println!("            ({low:#04x}, \"{escaped}\"),");
        }
        println!("        ],");
    }
    println!("        _ => &[],\n    }}\n}}");
}
