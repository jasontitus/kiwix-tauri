// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Offline native-dialog and sharing-page localization with bundled CLDR data.
use icu_decimal::{input::Decimal, DecimalFormatter};
use icu_locale::{Locale, LocaleDirectionality, LocaleExpander};
use icu_plurals::{PluralCategory, PluralRules};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

#[derive(RustEmbed)]
#[folder = "../src/locales/messages/"]
struct Messages;
#[derive(Deserialize)]
struct LocaleEntry {
    code: String,
}
#[derive(Deserialize)]
struct Metadata {
    aliases: HashMap<String, String>,
    locales: Vec<LocaleEntry>,
}
fn metadata() -> &'static Metadata {
    static DATA: LazyLock<Metadata> = LazyLock::new(|| {
        serde_json::from_str(include_str!("../../src/locales/locale-index.json"))
            .expect("valid bundled locale index")
    });
    &DATA
}
struct Catalog {
    messages: HashMap<String, Value>,
    plurals: PluralRules,
    numbers: DecimalFormatter,
}
fn catalog(code: &str) -> Arc<Catalog> {
    static CACHE: LazyLock<Mutex<HashMap<String, Arc<Catalog>>>> = LazyLock::new(Default::default);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .entry(code.to_owned())
        .or_insert_with(|| {
            let locale: Locale = code.parse().expect("valid bundled locale code");
            let data = Messages::get(&format!("{code}.json")).expect("bundled locale catalog");
            Arc::new(Catalog {
                messages: serde_json::from_slice(&data.data).expect("valid bundled catalog"),
                plurals: PluralRules::try_new_cardinal(locale.clone().into())
                    .expect("bundled CLDR plural data"),
                numbers: DecimalFormatter::try_new(locale.into(), Default::default())
                    .expect("bundled CLDR decimal data"),
            })
        })
        .clone()
}
fn resolve(request: &str) -> Option<String> {
    let request = request.trim().replace('_', "-");
    let lower = request.to_ascii_lowercase();
    let aliased = if let Some(alias) = metadata().aliases.get(&lower) {
        alias.clone()
    } else if let Some((base, suffix)) = lower.split_once('-') {
        metadata()
            .aliases
            .get(base)
            .map(|alias| format!("{alias}-{suffix}"))
            .unwrap_or(request)
    } else {
        request
    };
    let requested: Locale = aliased.parse().ok()?;
    let exact = requested.to_string();
    if let Some(entry) = metadata()
        .locales
        .iter()
        .find(|entry| entry.code.eq_ignore_ascii_case(&exact))
    {
        return Some(entry.code.clone());
    }
    let expander = LocaleExpander::new_extended();
    let mut expanded = requested.id.clone();
    expander.maximize(&mut expanded);
    // A region may imply a script (zh-TW); never discard it before matching.
    let compatible = |code: &str| {
        let Ok(mut locale) = code.parse::<Locale>() else {
            return false;
        };
        expander.maximize(&mut locale.id);
        locale.id.language == expanded.language && locale.id.script == expanded.script
    };
    let script = format!("{}-{}", expanded.language, expanded.script?);
    for candidate in [script, requested.id.language.to_string()] {
        if let Some(entry) = metadata()
            .locales
            .iter()
            .find(|entry| entry.code.eq_ignore_ascii_case(&candidate) && compatible(&entry.code))
        {
            return Some(entry.code.clone());
        }
    }
    metadata()
        .locales
        .iter()
        .find(|entry| compatible(&entry.code))
        .map(|entry| entry.code.clone())
}

pub struct Localizer {
    code: String,
    current: Arc<Catalog>,
    english: Arc<Catalog>,
    rtl: bool,
}
impl Localizer {
    pub fn for_app(setting: &str) -> Self {
        Self::new(
            setting,
            &sys_locale::get_locale().unwrap_or_else(|| "en".into()),
        )
    }
    pub fn new(setting: &str, accept_language: &str) -> Self {
        let code = if setting == "system" {
            let mut preferences: Vec<_> = accept_language
                .split(',')
                .enumerate()
                .filter_map(|(order, part)| {
                    let mut pieces = part.trim().split(';');
                    let range = pieces.next()?.trim();
                    let mut quality = 1000u16;
                    for parameter in pieces {
                        let (name, value) = parameter.trim().split_once('=')?;
                        if !name.eq_ignore_ascii_case("q") {
                            return None;
                        }
                        // RFC 9110 qvalues have at most three fractional digits.
                        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
                        if fraction.len() > 3 || !fraction.bytes().all(|c| c.is_ascii_digit()) {
                            return None;
                        }
                        quality = match whole {
                            "0" => {
                                fraction.parse::<u16>().unwrap_or(0)
                                    * 10u16.pow((3 - fraction.len()) as u32)
                            }
                            "1" if fraction.bytes().all(|c| c == b'0') => 1000,
                            _ => return None,
                        };
                    }
                    (quality > 0).then_some((quality, order, range))
                })
                .collect();
            preferences.sort_by_key(|(quality, order, _)| (std::cmp::Reverse(*quality), *order));
            preferences
                .into_iter()
                .find_map(|(_, _, range)| {
                    if range == "*" {
                        Some("en".into())
                    } else {
                        resolve(range)
                    }
                })
                .unwrap_or_else(|| "en".into())
        } else {
            resolve(setting).unwrap_or_else(|| "en".into())
        };
        let locale: Locale = code.parse().expect("valid resolved locale");
        let rtl = LocaleDirectionality::new_extended().is_right_to_left(&locale.id);
        Self {
            current: catalog(&code),
            english: catalog("en"),
            code,
            rtl,
        }
    }
    pub fn lang(&self) -> &str {
        &self.code
    }
    pub fn dir(&self) -> &str {
        if self.rtl {
            "rtl"
        } else {
            "ltr"
        }
    }
    pub fn text(&self, source: &str, values: &[(&str, &str)]) -> String {
        self.render(source, None, values, &[])
    }
    pub fn counted(&self, source: &str, count: u64, shown: u64) -> String {
        self.render(
            source,
            Some(count),
            &[],
            &[("count", count), ("shown", shown)],
        )
    }
    fn render(
        &self,
        source: &str,
        count: Option<u64>,
        values: &[(&str, &str)],
        numbers: &[(&str, u64)],
    ) -> String {
        fn choose<'a>(catalog: &'a Catalog, source: &str, count: Option<u64>) -> Option<&'a str> {
            let message = catalog.messages.get(source)?;
            if let Some(text) = message.as_str() {
                return Some(text);
            }
            let category = match catalog.plurals.category_for(count?) {
                PluralCategory::Zero => "zero",
                PluralCategory::One => "one",
                PluralCategory::Two => "two",
                PluralCategory::Few => "few",
                PluralCategory::Many => "many",
                PluralCategory::Other => "other",
            };
            message
                .get(category)
                .or_else(|| message.get("other"))?
                .as_str()
        }
        // Missing translations switch BOTH grammar and number formatting to English.
        let (template, grammar) = if let Some(text) = choose(&self.current, source, count) {
            (text, &self.current)
        } else {
            (
                choose(&self.english, source, count).unwrap_or(source),
                &self.english,
            )
        };
        let formatted: Vec<_> = numbers
            .iter()
            .map(|(name, number)| {
                (
                    *name,
                    grammar.numbers.format_to_string(&Decimal::from(*number)),
                )
            })
            .collect();
        // Single-pass replacement: substituted user text cannot introduce new placeholders.
        let mut output = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            output.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}') else {
                output.push_str(&rest[start..]);
                return output;
            };
            let key = &rest[start + 1..start + end];
            if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
                output.push_str(value);
            } else if let Some((_, value)) = formatted.iter().find(|(name, _)| *name == key) {
                output.push_str(value);
            } else {
                output.push_str(&rest[start..=start + end]);
            }
            rest = &rest[start + end + 1..];
        }
        output.push_str(rest);
        output
    }
}
