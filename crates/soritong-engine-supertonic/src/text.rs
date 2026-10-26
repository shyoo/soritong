//! Text handling ported from upstream `rust/src/helper.rs` (identical to its Python helper).
//!
//! The model reads Unicode code points through `unicode_indexer.json`. Precomposed Hangul syllables
//! are *not* in that table, so NFKD (splitting each syllable into conjoining jamo) is required, not
//! cosmetic. Digits are passed through as digits: upstream has no number normaliser, and Supertonic
//! 3 reads them itself.

use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

static EMOJI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\x{1F600}-\x{1F64F}\x{1F300}-\x{1F5FF}\x{1F680}-\x{1F6FF}\x{1F700}-\x{1F77F}\x{1F780}-\x{1F7FF}\x{1F800}-\x{1F8FF}\x{1F900}-\x{1F9FF}\x{1FA00}-\x{1FA6F}\x{1FA70}-\x{1FAFF}\x{2600}-\x{26FF}\x{2700}-\x{27BF}\x{1F1E6}-\x{1F1FF}]+").unwrap()
});
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static ENDS_WITH_PUNCT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"[.!?;:,'"\u{201C}\u{201D}\u{2018}\u{2019})\]}…。」』】〉》›»]$"#).unwrap()
});
static PARAGRAPH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\s*\n").unwrap());
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([.!?])\s+").unwrap());

/// Normalises `text` and wraps it in the model's language tags, e.g. `<ko>…</ko>`.
pub fn preprocess(text: &str, lang: &str) -> String {
    let mut text: String = text.nfkd().collect();
    text = EMOJI.replace_all(&text, "").into_owned();
    for (from, to) in [
        ("–", "-"),
        ("‑", "-"),
        ("—", "-"),
        ("_", " "),
        ("\u{201C}", "\""),
        ("\u{201D}", "\""),
        ("\u{2018}", "'"),
        ("\u{2019}", "'"),
        ("´", "'"),
        ("`", "'"),
        ("[", " "),
        ("]", " "),
        ("|", " "),
        ("/", " "),
        ("#", " "),
        ("→", " "),
        ("←", " "),
    ] {
        text = text.replace(from, to);
    }
    for symbol in ["♥", "☆", "♡", "©", "\\"] {
        text = text.replace(symbol, "");
    }
    for (from, to) in [
        ("@", " at "),
        ("e.g.,", "for example, "),
        ("i.e.,", "that is, "),
    ] {
        text = text.replace(from, to);
    }
    for p in [",", ".", "!", "?", ";", ":", "'"] {
        text = text.replace(&format!(" {p}"), p);
    }
    for q in ["\"", "'", "`"] {
        let double = q.repeat(2);
        while text.contains(&double) {
            text = text.replace(&double, q);
        }
    }
    text = SPACES.replace_all(&text, " ").trim().to_string();
    if !text.is_empty() && !ENDS_WITH_PUNCT.is_match(&text) {
        text.push('.');
    }
    format!("<{lang}>{text}</{lang}>")
}

/// Maps code points to model token ids. Characters the model has no id for (hanja, rare
/// symbols…) are dropped: upstream passes `-1` through, which the embedding cannot look up.
pub fn to_ids(text: &str, indexer: &[i64]) -> Vec<i64> {
    text.chars()
        .filter_map(|c| indexer.get(c as usize).copied().filter(|&id| id >= 0))
        .collect()
}

/// Upstream's long-input guidance: chunks of at most 120 characters for Korean/Japanese and 300
/// otherwise, split by paragraph, then sentence, then comma, then word. Lengths are in characters
/// as in the Python reference; upstream's Rust port counts UTF-8 bytes, which would cut Korean
/// chunks to a third.
pub fn max_chunk_len(lang: &str) -> usize {
    if lang == "ko" || lang == "ja" {
        120
    } else {
        300
    }
}

const ABBREVIATIONS: &[&str] = &[
    "Dr.", "Mr.", "Mrs.", "Ms.", "Prof.", "Sr.", "Jr.", "St.", "Ave.", "Rd.", "Blvd.", "Dept.",
    "Inc.", "Ltd.", "Co.", "Corp.", "etc.", "vs.", "i.e.", "e.g.", "Ph.D.",
];

fn len(s: &str) -> usize {
    s.chars().count()
}

pub fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    for para in PARAGRAPH.split(text.trim()) {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if len(para) <= max_len {
            chunks.push(para.to_string());
            continue;
        }
        let mut current = String::new();
        let flush = |current: &mut String, chunks: &mut Vec<String>| {
            if !current.trim().is_empty() {
                chunks.push(current.trim().to_string());
            }
            current.clear();
        };
        for sentence in split_sentences(para) {
            let sentence = sentence.trim();
            if sentence.is_empty() {
                continue;
            }
            if len(sentence) > max_len {
                flush(&mut current, &mut chunks);
                for part in sentence.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    if len(part) > max_len {
                        let mut words = String::new();
                        for word in part.split_whitespace() {
                            if !words.is_empty() && len(&words) + len(word) + 1 > max_len {
                                flush(&mut words, &mut chunks);
                            }
                            if !words.is_empty() {
                                words.push(' ');
                            }
                            words.push_str(word);
                        }
                        flush(&mut words, &mut chunks);
                    } else {
                        if !current.is_empty() && len(&current) + len(part) + 1 > max_len {
                            flush(&mut current, &mut chunks);
                        }
                        if !current.is_empty() {
                            current.push_str(", ");
                        }
                        current.push_str(part);
                    }
                }
                continue;
            }
            if !current.is_empty() && len(&current) + len(sentence) + 1 > max_len {
                flush(&mut current, &mut chunks);
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(sentence);
        }
        flush(&mut current, &mut chunks);
    }
    chunks
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut last = 0;
    for m in SENTENCE_END.find_iter(text) {
        let candidate = format!(
            "{}{}",
            text[last..m.start()].trim(),
            &text[m.start()..m.start() + 1]
        );
        if !ABBREVIATIONS.iter().any(|a| candidate.ends_with(a)) {
            sentences.push(text[last..m.end()].to_string());
            last = m.end();
        }
    }
    if last < text.len() {
        sentences.push(text[last..].to_string());
    }
    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn korean_is_decomposed_to_jamo_and_tagged() {
        let p = preprocess("안녕하세요", "ko");
        assert!(p.starts_with("<ko>") && p.ends_with(".</ko>"));
        // 5 syllables -> 2 or 3 jamo each; no precomposed syllable survives.
        assert!(!p.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)));
        assert!(p.chars().any(|c| ('\u{1100}'..='\u{11FF}').contains(&c)));
        let body: String = p
            .trim_start_matches("<ko>")
            .trim_end_matches("</ko>")
            .into();
        assert_eq!(body.nfc().collect::<String>(), "안녕하세요.");
    }

    #[test]
    fn mixed_text_numbers_and_symbols() {
        assert_eq!(
            preprocess("저는  Harry Potter 3권을 좋아해요 !", "ko")
                .nfc()
                .collect::<String>(),
            "<ko>저는 Harry Potter 3권을 좋아해요!</ko>"
        );
        assert_eq!(
            preprocess("“Quotes” — and 3.5/10 #rating 😀", "en"),
            "<en>\"Quotes\" - and 3.5 10 rating.</en>"
        );
        assert_eq!(preprocess("me@home", "en"), "<en>me at home.</en>");
        assert_eq!(
            preprocess("끝났나요?", "ko").nfc().collect::<String>(),
            "<ko>끝났나요?</ko>"
        );
        assert_eq!(preprocess("   ", "en"), "<en></en>");
    }

    #[test]
    fn unknown_characters_are_dropped_from_ids() {
        let mut indexer = vec![-1i64; 128];
        indexer['a' as usize] = 7;
        indexer['b' as usize] = 8;
        assert_eq!(to_ids("a漢b", &indexer), vec![7, 8]);
    }

    #[test]
    fn chunks_long_text_like_upstream() {
        assert_eq!(chunk_text("짧은 문장.", 120), vec!["짧은 문장."]);
        assert!(chunk_text("  \n\n ", 120).is_empty());
        assert_eq!(
            chunk_text("첫 문단.\n\n둘째 문단.", 120),
            vec!["첫 문단.", "둘째 문단."]
        );

        let long = "오늘 책 정말 재밌었어요. ".repeat(12);
        let chunks = chunk_text(&long, max_chunk_len("ko"));
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.chars().count() <= 120));
        assert!(
            chunks[0].chars().count() > 100,
            "chunks are measured in characters"
        );
        assert_eq!(chunks.join(" "), long.trim());

        // Abbreviations do not end a sentence.
        let en = format!("Dr. Kim said hello. {}", "word ".repeat(70));
        let chunks = chunk_text(&en, 300);
        assert!(chunks[0].starts_with("Dr. Kim said hello."));
        assert!(chunks.iter().all(|c| c.len() <= 300));
    }
}
