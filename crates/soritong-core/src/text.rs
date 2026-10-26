//! Text helpers: language detection and sentence splitting.
//!
//! Splitting lets the first sentence play while the next is synthesised, which is most of the
//! perceived latency on non-streaming engines.

use crate::Lang;

fn is_hangul(c: char) -> bool {
    matches!(c,
        '\u{AC00}'..='\u{D7A3}'   // syllables
        | '\u{1100}'..='\u{11FF}' // jamo
        | '\u{3130}'..='\u{318F}' // compatibility jamo (ㅋㅋ)
    )
}

/// Korean if a meaningful share of the letters are Hangul, else English.
pub fn detect_lang(text: &str) -> Lang {
    let (mut hangul, mut latin) = (0usize, 0usize);
    for c in text.chars() {
        if is_hangul(c) {
            hangul += 1;
        } else if c.is_ascii_alphabetic() {
            latin += 1;
        }
    }
    // A Hangul syllable carries roughly three Latin letters' worth of sound.
    if hangul * 3 >= latin && hangul > 0 {
        Lang::Ko
    } else {
        Lang::En
    }
}

/// Splits on sentence-ending punctuation (`. ! ? …` and their full-width forms) and newlines,
/// keeping the punctuation. Decimal points like `3.5` are not split. Empty pieces are dropped.
pub fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            push_trimmed(&mut out, &mut cur);
            i += 1;
            continue;
        }
        cur.push(c);
        let ends = matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？');
        let decimal = c == '.'
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if ends && !decimal {
            // Keep runs like "?!" or "..." together.
            while let Some(&n) = chars.get(i + 1) {
                if matches!(n, '.' | '!' | '?' | '…' | '"' | '\'' | ')' | '”' | '’') {
                    cur.push(n);
                    i += 1;
                } else {
                    break;
                }
            }
            push_trimmed(&mut out, &mut cur);
        }
        i += 1;
    }
    push_trimmed(&mut out, &mut cur);
    out
}

fn push_trimmed(out: &mut Vec<String>, cur: &mut String) {
    let t = cur.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    cur.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_korean_and_english() {
        assert_eq!(detect_lang("오늘 책 정말 재밌었어요"), Lang::Ko);
        assert_eq!(detect_lang("I agree with that point."), Lang::En);
        assert_eq!(detect_lang("ㅋㅋㅋ"), Lang::Ko);
        assert_eq!(detect_lang("저는 Harry Potter 좋아해요"), Lang::Ko);
        assert_eq!(detect_lang("12345"), Lang::En);
    }

    #[test]
    fn splits_sentences_keeping_punctuation() {
        assert_eq!(
            split_sentences("안녕하세요. 반가워요! 잘 지냈어요?"),
            vec!["안녕하세요.", "반가워요!", "잘 지냈어요?"]
        );
        assert_eq!(split_sentences("정말요?! 와..."), vec!["정말요?!", "와..."]);
        assert_eq!(
            split_sentences("3.5점 줄게요. 좋아요"),
            vec!["3.5점 줄게요.", "좋아요"]
        );
        assert_eq!(
            split_sentences("첫 줄\n\n둘째 줄"),
            vec!["첫 줄", "둘째 줄"]
        );
        assert!(split_sentences("   ").is_empty());
    }
}
