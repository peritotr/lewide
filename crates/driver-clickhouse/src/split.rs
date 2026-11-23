//! Statement boundaries in editor text. ClickHouse's HTTP interface takes one
//! statement per request, so the text is cut on semicolons that are not inside
//! a string, a quoted name or a comment.

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
            quote @ (b'\'' | b'"' | b'`') => i = past_quoted(bytes, i, quote),
            b'-' if bytes.get(i + 1) == Some(&b'-') => i = line_end(bytes, i),
            b'#' => i = line_end(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = past_block_comment(bytes, i),
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

/// Past the closing quote. A backslash escapes the next character, and a
/// doubled quote stands for one.
fn past_quoted(bytes: &[u8], open: usize, quote: u8) -> usize {
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            c if c == quote && bytes.get(i + 1) == Some(&quote) => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |p| from + p)
}

/// ClickHouse nests block comments.
fn past_block_comment(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0;
    let mut i = open;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// Whether anything besides blanks and comments is left.
fn has_code(piece: &str) -> bool {
    let bytes = piece.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'-' if bytes.get(i + 1) == Some(&b'-') => i = line_end(bytes, i),
            b'#' => i = line_end(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = past_block_comment(bytes, i),
            _ => return true,
        }
    }
    false
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
            texts("SELECT 'a;\\'b', \"c;d\", `e;f` -- g;h\n; SELECT 2 # i;j\n"),
            ["SELECT 'a;\\'b', \"c;d\", `e;f` -- g;h", "SELECT 2 # i;j"]
        );
        assert_eq!(
            texts("SELECT /* a /* b; */ c; */ 1"),
            ["SELECT /* a /* b; */ c; */ 1"]
        );
        assert!(texts("  ; -- only a comment\n;").is_empty());
    }

    #[test]
    fn keeps_where_each_statement_starts() {
        let found = statements("SELECT 1;\n  SELECT x");
        assert_eq!(found[1].offset, 12);
        assert_eq!(found[1].text, "SELECT x");
    }
}
