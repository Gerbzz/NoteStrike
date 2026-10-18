//! Find / replace and cursor-position helpers. All ranges are byte ranges into
//! the searched `&str` and always fall on `char` boundaries.

use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub wrap_around: bool,
}

fn fold(c: char, match_case: bool) -> char {
    if match_case {
        return c;
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// If `needle` matches at byte `start`, return the end byte of the match.
fn match_at(hay: &str, start: usize, needle: &str, opts: FindOptions) -> Option<usize> {
    let mut hay_chars = hay[start..].char_indices();
    let mut end = start;
    for n in needle.chars() {
        let (offset, h) = hay_chars.next()?;
        if fold(h, opts.match_case) != fold(n, opts.match_case) {
            return None;
        }
        end = start + offset + h.len_utf8();
    }
    if opts.whole_word {
        let before = hay[..start].chars().next_back();
        let after = hay[end..].chars().next();
        if before.is_some_and(is_word_char) || after.is_some_and(is_word_char) {
            return None;
        }
    }
    Some(end)
}

fn find_in(
    hay: &str,
    needle: &str,
    from: usize,
    to: usize,
    opts: FindOptions,
) -> Option<Range<usize>> {
    let mut index = from;
    while index <= to && index < hay.len() {
        if let Some(end) = match_at(hay, index, needle, opts) {
            return Some(index..end);
        }
        index += hay[index..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// First match starting at or after byte `from`, optionally wrapping to the start.
pub fn find_next(hay: &str, needle: &str, from: usize, opts: FindOptions) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let from = floor_boundary(hay, from.min(hay.len()));
    find_in(hay, needle, from, hay.len(), opts).or_else(|| {
        if opts.wrap_around && from > 0 {
            find_in(hay, needle, 0, from, opts)
        } else {
            None
        }
    })
}

/// Last match ending at or before byte `before`, optionally wrapping to the end.
pub fn find_prev(
    hay: &str,
    needle: &str,
    before: usize,
    opts: FindOptions,
) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let before = floor_boundary(hay, before.min(hay.len()));
    let last_before = |limit: usize| {
        let mut found = None;
        let mut index = 0;
        while index < hay.len() {
            if let Some(end) = match_at(hay, index, needle, opts) {
                if end <= limit {
                    found = Some(index..end);
                } else {
                    break;
                }
            }
            index += hay[index..].chars().next().map_or(1, char::len_utf8);
        }
        found
    };
    last_before(before).or_else(|| {
        if opts.wrap_around {
            last_before(hay.len()).filter(|range| range.end > before)
        } else {
            None
        }
    })
}

pub fn count_matches(hay: &str, needle: &str, opts: FindOptions) -> usize {
    let mut count = 0;
    let mut from = 0;
    let no_wrap = FindOptions {
        wrap_around: false,
        ..opts
    };
    while let Some(range) = find_next(hay, needle, from, no_wrap) {
        count += 1;
        from = range.end.max(range.start + 1);
    }
    count
}

/// Replace every match; returns the new text and the number of replacements.
pub fn replace_all(
    hay: &str,
    needle: &str,
    replacement: &str,
    opts: FindOptions,
) -> (String, usize) {
    if needle.is_empty() {
        return (hay.to_owned(), 0);
    }
    let no_wrap = FindOptions {
        wrap_around: false,
        ..opts
    };
    let mut out = String::with_capacity(hay.len());
    let mut copied = 0;
    let mut count = 0;
    while let Some(range) = find_next(hay, needle, copied, no_wrap) {
        out.push_str(&hay[copied..range.start]);
        out.push_str(replacement);
        copied = range.end;
        count += 1;
    }
    out.push_str(&hay[copied..]);
    (out, count)
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte, _)| byte)
}

pub fn byte_to_char(text: &str, byte_index: usize) -> usize {
    text[..floor_boundary(text, byte_index.min(text.len()))]
        .chars()
        .count()
}

/// 1-based (line, column) of a char index, counting columns in characters.
pub fn line_col(text: &str, char_index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for c in text.chars().take(char_index) {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

pub fn line_count(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count() + 1
}

/// Char index of the start of 1-based `line`, or `None` if out of range.
pub fn line_start_char(text: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    if line == 1 {
        return Some(0);
    }
    let mut current = 1;
    for (index, c) in text.chars().enumerate() {
        if c == '\n' {
            current += 1;
            if current == line {
                return Some(index + 1);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_replace_basics() {
        let opts = FindOptions {
            wrap_around: true,
            ..Default::default()
        };
        let text = "Cat cat CAT concat";
        assert_eq!(find_next(text, "cat", 1, opts), Some(4..7));
        assert_eq!(count_matches(text, "cat", opts), 4);
        let whole = FindOptions {
            whole_word: true,
            ..opts
        };
        assert_eq!(count_matches(text, "cat", whole), 3);
        let exact = FindOptions {
            match_case: true,
            ..opts
        };
        assert_eq!(
            replace_all(text, "cat", "dog", exact),
            ("Cat dog CAT condog".into(), 2)
        );
        assert_eq!(find_prev(text, "cat", 7, opts), Some(4..7));
        assert_eq!(find_prev(text, "cat", 2, opts), Some(15..18));
        assert_eq!(find_next("ÄÖ ä", "ä", 1, opts), Some(0..2));
        assert_eq!(line_col("ab\ncd", 4), (2, 2));
        assert_eq!(line_start_char("ab\ncd\n", 3), Some(6));
        assert_eq!(line_start_char("ab", 3), None);
    }
}
