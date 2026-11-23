//! Statement boundaries in editor text (the copy in driver-duckdb, whose SQL
//! quotes the same way). InfluxDB takes one statement per request, so the
//! text is cut on semicolons that are not inside a string, a quoted name, a
//! dollar-quoted string or a comment.

/// One statement and the byte offset where it starts in the whole text.
#[derive(Debug, PartialEq, Eq)]
pub struct Piece<'a> {
    pub text: &'a str,
    pub offset: usize,
}

pub fn statements<'a>(text: &'a str) -> Vec<Piece<'a>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let push = |out: &mut Vec<Piece<'a>>, from: usize, to: usize| {
        let piece = &text[from..to];
        if has_code(piece) {
            let lead = piece.len() - piece.trim_start().len();
            out.push(Piece {
                text: piece.trim(),
                offset: from + lead,
            });
        }
    };
    while i < bytes.len() {
        match bytes[i] {
            // E'…' is the one string form where a backslash escapes.
            b'\'' => {
                let escapes = i > 0
                    && matches!(bytes[i - 1], b'E' | b'e')
                    && (i < 2 || !is_word(bytes[i - 2]));
                i = past_quoted(bytes, i, b'\'', escapes);
            }
            b'"' => i = past_quoted(bytes, i, b'"', false),
            b'$' => i = past_dollar_quoted(bytes, i),
            b'-' if bytes.get(i + 1) == Some(&b'-') => i = line_end(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = text[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |p| i + 2 + p + 2);
            }
            b';' => {
                push(&mut out, start, i);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    push(&mut out, start, bytes.len());
    out
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Past the closing quote; a doubled quote stands for one.
fn past_quoted(bytes: &[u8], open: usize, quote: u8, escapes: bool) -> usize {
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if escapes => i += 2,
            c if c == quote && bytes.get(i + 1) == Some(&quote) => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// `$$ … $$` or `$tag$ … $tag$`; a `$` that does not start one is passed over.
fn past_dollar_quoted(bytes: &[u8], open: usize) -> usize {
    if open > 0 && is_word(bytes[open - 1]) {
        return open + 1;
    }
    let tag_end = bytes[open + 1..]
        .iter()
        .position(|&b| !(b.is_ascii_alphabetic() || b == b'_'))
        .map(|p| open + 1 + p);
    let Some(tag_end) = tag_end.filter(|&e| bytes[e] == b'$') else {
        return open + 1;
    };
    let tag = &bytes[open..=tag_end];
    let mut i = tag_end + 1;
    while i + tag.len() <= bytes.len() {
        if &bytes[i..i + tag.len()] == tag {
            return i + tag.len();
        }
        i += 1;
    }
    bytes.len()
}

fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |p| from + p)
}

/// Whether anything besides blanks and comments is left.
fn has_code(piece: &str) -> bool {
    let bytes = piece.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'-' if bytes.get(i + 1) == Some(&b'-') => i = line_end(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = piece[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |p| i + 2 + p + 2);
            }
            _ => return true,
        }
    }
    false
}

/// The first word, upper-cased, after leading comments.
pub fn first_word(statement: &str) -> String {
    let mut rest = statement.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    rest.split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("")
        .to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(text: &str) -> Vec<&str> {
        statements(text).into_iter().map(|p| p.text).collect()
    }

    #[test]
    fn cuts_on_real_semicolons_only() {
        assert_eq!(texts("SELECT 1; SELECT 2;"), ["SELECT 1", "SELECT 2"]);
        assert_eq!(
            texts("SELECT 'a;''b\\', \"c;d\" -- e;f\n; SELECT $$g;h$$, $t$i;$t$, E'j\\';k'"),
            [
                "SELECT 'a;''b\\', \"c;d\" -- e;f",
                "SELECT $$g;h$$, $t$i;$t$, E'j\\';k'"
            ]
        );
        assert_eq!(
            texts("SELECT $1; SELECT /* ; */ 2"),
            ["SELECT $1", "SELECT /* ; */ 2"]
        );
        assert!(texts(" ; -- nothing\n").is_empty());
        assert_eq!(statements("SELECT 1;\n  SELECT x")[1].offset, 12);
    }

    #[test]
    fn reads_the_first_word() {
        assert_eq!(
            first_word("-- note\n /* x */ insert into t values (1)"),
            "INSERT"
        );
        assert_eq!(first_word("  BEGIN TRANSACTION"), "BEGIN");
        assert_eq!(first_word(""), "");
    }
}
