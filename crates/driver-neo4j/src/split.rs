//! Splits editor text into single Cypher statements.
//!
//! The server's Query API accepts exactly one statement per request, so text
//! with several statements separated by `;` is cut up here.

/// One statement and where it started in the whole text, so an error position
/// the server gives for the piece can be moved back into the text.
#[derive(Debug, PartialEq)]
pub struct Piece<'a> {
    pub text: &'a str,
    /// Characters before the piece.
    pub char_offset: u32,
    /// Line breaks before the piece.
    pub line_offset: u32,
}

/// Cuts at every `;` that is not inside a string, a backtick-quoted name or a
/// comment. Pieces with nothing but white space and comments are dropped.
pub fn statements(text: &str) -> Vec<Piece<'_>> {
    #[derive(PartialEq)]
    enum State {
        Code,
        /// Inside a string or quoted name that ends at this character.
        Quoted(char),
        LineComment,
        BlockComment,
    }

    let mut pieces = Vec::new();
    let mut state = State::Code;
    // Where the current piece began: byte index, characters and lines before it.
    let (mut start, mut start_chars, mut start_lines) = (0, 0u32, 0u32);
    let (mut chars_seen, mut lines_seen) = (0u32, 0u32);
    let mut has_code = false;

    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        chars_seen += 1;
        if c == '\n' {
            lines_seen += 1;
        }
        let next = chars.peek().map(|&(_, n)| n);
        match state {
            State::Code => match c {
                ';' => {
                    if has_code {
                        pieces.push(Piece {
                            text: &text[start..at],
                            char_offset: start_chars,
                            line_offset: start_lines,
                        });
                    }
                    start = at + 1;
                    start_chars = chars_seen;
                    start_lines = lines_seen;
                    has_code = false;
                }
                '\'' | '"' | '`' => {
                    state = State::Quoted(c);
                    has_code = true;
                }
                '/' if next == Some('/') => state = State::LineComment,
                '/' if next == Some('*') => {
                    chars.next();
                    chars_seen += 1;
                    state = State::BlockComment;
                }
                c if !c.is_whitespace() => has_code = true,
                _ => {}
            },
            State::Quoted(quote) => {
                if c == '\\' && quote != '`' {
                    // The escaped character cannot end the string.
                    if let Some((_, escaped)) = chars.next() {
                        chars_seen += 1;
                        if escaped == '\n' {
                            lines_seen += 1;
                        }
                    }
                } else if c == quote {
                    // A doubled backtick inside a quoted name is re-entered
                    // on the next character, which gives the same result.
                    state = State::Code;
                }
            }
            State::LineComment => {
                if c == '\n' {
                    state = State::Code;
                }
            }
            State::BlockComment => {
                if c == '*' && next == Some('/') {
                    chars.next();
                    chars_seen += 1;
                    state = State::Code;
                }
            }
        }
    }
    if has_code {
        pieces.push(Piece {
            text: &text[start..],
            char_offset: start_chars,
            line_offset: start_lines,
        });
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(text: &str) -> Vec<&str> {
        statements(text).into_iter().map(|p| p.text).collect()
    }

    #[test]
    fn splits_on_semicolons_outside_quotes_and_comments() {
        assert_eq!(texts("RETURN 1"), ["RETURN 1"]);
        assert_eq!(texts("RETURN 1; RETURN 2;"), ["RETURN 1", " RETURN 2"]);
        assert_eq!(
            texts("RETURN ';' AS a; RETURN 2"),
            ["RETURN ';' AS a", " RETURN 2"]
        );
        assert_eq!(texts(r#"RETURN "a\";b" AS a"#), [r#"RETURN "a\";b" AS a"#]);
        assert_eq!(texts(r"RETURN 'it\'s; fine'"), [r"RETURN 'it\'s; fine'"]);
        assert_eq!(
            texts("MATCH (n:`a;``b`) RETURN n"),
            ["MATCH (n:`a;``b`) RETURN n"]
        );
        assert_eq!(
            texts("RETURN 1 // a; b\n; RETURN 2"),
            ["RETURN 1 // a; b\n", " RETURN 2"]
        );
        assert_eq!(texts("RETURN /* ; */ 1"), ["RETURN /* ; */ 1"]);
    }

    #[test]
    fn drops_pieces_without_code() {
        assert!(texts("").is_empty());
        assert!(texts(" ;\n; // only a comment\n /* and another */ ").is_empty());
        assert_eq!(texts(";;RETURN 1;;"), ["RETURN 1"]);
    }

    #[test]
    fn records_where_each_piece_starts() {
        let pieces = statements("RETURN 'é';\nRETURN\n2");
        assert_eq!(pieces[0].char_offset, 0);
        assert_eq!(pieces[0].line_offset, 0);
        assert_eq!(pieces[1].text, "\nRETURN\n2");
        // Counted in characters, not bytes: `é` is one.
        assert_eq!(pieces[1].char_offset, 11);
        assert_eq!(pieces[1].line_offset, 0);

        let pieces = statements("RETURN 1\n;\n\nRETURN 2");
        assert_eq!(pieces[1].line_offset, 1);
        assert_eq!(pieces[1].char_offset, 10);
    }
}
