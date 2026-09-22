//! `ut`と`tut`の表記ゆれ検索を共通化する。

use unicode_normalization::UnicodeNormalization;

pub(in crate::commands) fn normalize_search_text(value: &str) -> String {
    value
        .trim()
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|character| match character {
            '\u{30a1}'..='\u{30f6}' => char::from_u32(character as u32 - 0x60).unwrap_or(character),
            '~' | '～' | '〜' => '〜',
            '－' | '−' | '‐' | '⁃' | '‑' | '‒' | '–' | '—' | '―' | '-' => 'ー',
            _ => character,
        })
        .collect()
}
