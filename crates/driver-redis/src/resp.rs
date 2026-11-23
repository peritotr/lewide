//! The Redis wire protocol (RESP2): commands go out as arrays of byte strings,
//! replies come back as one of five kinds. Small enough to write here rather
//! than depend on a client library.

use std::future::Future;
use std::pin::Pin;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Simple(String),
    Error(String),
    Int(i64),
    /// `None` is the protocol's null string.
    Bulk(Option<Vec<u8>>),
    /// `None` is the protocol's null array.
    Array(Option<Vec<Reply>>),
}

/// A command as the bytes to send.
pub fn encode(args: &[Vec<u8>]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", args.len()).into_bytes();
    for arg in args {
        out.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        out.extend_from_slice(arg);
        out.extend_from_slice(b"\r\n");
    }
    out
}

fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

async fn line<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<String> {
    let mut text = String::new();
    if reader.read_line(&mut text).await? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "the server closed the connection",
        ));
    }
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

/// Reads one reply, however deeply its arrays nest.
pub fn read<'a, R: AsyncBufRead + Unpin + Send>(
    reader: &'a mut R,
) -> Pin<Box<dyn Future<Output = std::io::Result<Reply>> + Send + 'a>> {
    Box::pin(async move {
        let head = line(reader).await?;
        let (kind, rest) = head.split_at(head.len().min(1));
        let number = || {
            rest.parse::<i64>()
                .map_err(|_| invalid(format!("not a number: {rest}")))
        };
        match kind {
            "+" => Ok(Reply::Simple(rest.to_owned())),
            "-" => Ok(Reply::Error(rest.to_owned())),
            ":" => Ok(Reply::Int(number()?)),
            "$" => {
                let length = number()?;
                if length < 0 {
                    return Ok(Reply::Bulk(None));
                }
                let mut bytes = vec![0; length as usize + 2];
                reader.read_exact(&mut bytes).await?;
                bytes.truncate(length as usize);
                Ok(Reply::Bulk(Some(bytes)))
            }
            "*" => {
                let length = number()?;
                if length < 0 {
                    return Ok(Reply::Array(None));
                }
                let mut items = Vec::with_capacity(length as usize);
                for _ in 0..length {
                    items.push(read(reader).await?);
                }
                Ok(Reply::Array(Some(items)))
            }
            other => Err(invalid(format!("unexpected reply type {other:?}"))),
        }
    })
}

/// Splits a line as redis-cli does: words, with single or double quotes
/// around anything that holds spaces. `\"`, `\\`, `\n`, `\r` and `\t` are
/// read inside double quotes.
pub fn split_command(text: &str) -> Result<Vec<Vec<u8>>, String> {
    let mut args = Vec::new();
    let mut current: Option<Vec<u8>> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if let Some(done) = current.take() {
                    args.push(done);
                }
            }
            '"' | '\'' => {
                let word = current.get_or_insert_with(Vec::new);
                let mut closed = false;
                while let Some(inner) = chars.next() {
                    if inner == c {
                        closed = true;
                        break;
                    }
                    let out = if inner == '\\' && c == '"' {
                        match chars.next() {
                            Some('n') => '\n',
                            Some('r') => '\r',
                            Some('t') => '\t',
                            Some(other) => other,
                            None => return Err("a quote is not closed".into()),
                        }
                    } else {
                        inner
                    };
                    word.extend_from_slice(out.encode_utf8(&mut [0; 4]).as_bytes());
                }
                if !closed {
                    return Err("a quote is not closed".into());
                }
            }
            other => current
                .get_or_insert_with(Vec::new)
                .extend_from_slice(other.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    if let Some(done) = current {
        args.push(done);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn parse(bytes: &[u8]) -> Reply {
        let mut reader = tokio::io::BufReader::new(bytes);
        read(&mut reader).await.unwrap()
    }

    #[tokio::test]
    async fn reads_every_reply_kind() {
        assert_eq!(parse(b"+OK\r\n").await, Reply::Simple("OK".into()));
        assert_eq!(parse(b"-ERR no\r\n").await, Reply::Error("ERR no".into()));
        assert_eq!(parse(b":-7\r\n").await, Reply::Int(-7));
        assert_eq!(
            parse(b"$5\r\nhe\r\nl\r\n").await,
            Reply::Bulk(Some(b"he\r\nl".to_vec()))
        );
        assert_eq!(parse(b"$-1\r\n").await, Reply::Bulk(None));
        assert_eq!(
            parse(b"*2\r\n$1\r\na\r\n*1\r\n:1\r\n").await,
            Reply::Array(Some(vec![
                Reply::Bulk(Some(b"a".to_vec())),
                Reply::Array(Some(vec![Reply::Int(1)]))
            ]))
        );
        assert_eq!(parse(b"*-1\r\n").await, Reply::Array(None));
    }

    #[test]
    fn encodes_and_splits_commands() {
        assert_eq!(
            encode(&[b"GET".to_vec(), b"a b".to_vec()]),
            b"*2\r\n$3\r\nGET\r\n$3\r\na b\r\n"
        );
        let words = |text: &str| {
            split_command(text)
                .unwrap()
                .into_iter()
                .map(|w| String::from_utf8(w).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(words("SET  key value"), ["SET", "key", "value"]);
        assert_eq!(
            words(r#"SET k "two words" 'it''s'"#),
            ["SET", "k", "two words", "its"]
        );
        assert_eq!(words(r#"SET k "a\"b\n""#), ["SET", "k", "a\"b\n"]);
        assert_eq!(words(r#"SET k """#), ["SET", "k", ""]);
        assert!(split_command(r#"SET k "open"#).is_err());
        assert!(words("   ").is_empty());
    }
}
