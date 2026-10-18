//! Line-ending detection, normalisation, and restoration.
//!
//! The editor works on `\n`-only text. The detected (dominant) line ending is
//! written back on save, so a CRLF file stays CRLF and an LF file stays LF.
//! Files with mixed endings are reported so the user knows they will be
//! unified on save.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineEnding {
    CrLf,
    Lf,
    Cr,
}

impl LineEnding {
    pub const ALL: [LineEnding; 3] = [LineEnding::CrLf, LineEnding::Lf, LineEnding::Cr];

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::CrLf => "Windows (CRLF)",
            LineEnding::Lf => "Unix (LF)",
            LineEnding::Cr => "Classic Mac (CR)",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            LineEnding::CrLf => "CRLF",
            LineEnding::Lf => "LF",
            LineEnding::Cr => "CR",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::CrLf => "\r\n",
            LineEnding::Lf => "\n",
            LineEnding::Cr => "\r",
        }
    }

    pub fn from_short(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|eol| eol.short() == id)
    }

    /// Line ending used for new, empty documents.
    pub fn platform_default() -> Self {
        if cfg!(windows) {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EolCounts {
    pub crlf: usize,
    pub lf: usize,
    pub cr: usize,
}

impl EolCounts {
    pub fn is_mixed(&self) -> bool {
        [self.crlf, self.lf, self.cr]
            .iter()
            .filter(|&&n| n > 0)
            .count()
            > 1
    }

    /// The most common line ending; ties prefer CRLF, then LF. `None` when the
    /// text has no line breaks at all.
    pub fn dominant(&self) -> Option<LineEnding> {
        if self.crlf == 0 && self.lf == 0 && self.cr == 0 {
            return None;
        }
        let mut best = (LineEnding::CrLf, self.crlf);
        for candidate in [(LineEnding::Lf, self.lf), (LineEnding::Cr, self.cr)] {
            if candidate.1 > best.1 {
                best = candidate;
            }
        }
        Some(best.0)
    }
}

pub fn count(text: &str) -> EolCounts {
    let mut counts = EolCounts::default();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                counts.crlf += 1;
                index += 1;
            }
            b'\r' => counts.cr += 1,
            b'\n' => counts.lf += 1,
            _ => {}
        }
        index += 1;
    }
    counts
}

/// Convert any mix of CRLF / CR / LF into `\n`.
pub fn normalize(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// Write `text` (any line endings) using `eol` throughout.
pub fn apply(text: &str, eol: LineEnding) -> String {
    let normalized = normalize(text);
    match eol {
        LineEnding::Lf => normalized,
        other => normalized.replace('\n', other.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_round_trips() {
        let text = "a\r\nb\r\nc\nd\re";
        let counts = count(text);
        assert_eq!(
            counts,
            EolCounts {
                crlf: 2,
                lf: 1,
                cr: 1
            }
        );
        assert!(counts.is_mixed());
        assert_eq!(counts.dominant(), Some(LineEnding::CrLf));
        assert_eq!(normalize(text), "a\nb\nc\nd\ne");
        assert_eq!(apply("x\ny", LineEnding::Cr), "x\ry");
        assert_eq!(count("none").dominant(), None);
    }
}
