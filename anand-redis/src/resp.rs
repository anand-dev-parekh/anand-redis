use std::io;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    SimpleString(String),
    Error(String),
    Integer(i64),
    BulkString(Vec<u8>),
    Null,
    Array(Vec<Value>),
}

pub fn serialize(value: &Value) -> Vec<u8> {
    match value {
        Value::SimpleString(s) => format!("+{}\r\n", s).into_bytes(),
        Value::Error(s) => format!("-{}\r\n", s).into_bytes(),
        Value::Integer(n) => format!(":{}\r\n", n).into_bytes(),
        Value::BulkString(b) => {
            let mut out = format!("${}\r\n", b.len()).into_bytes();
            out.extend_from_slice(b);
            out.extend_from_slice(b"\r\n");
            out
        }
        Value::Null => b"$-1\r\n".to_vec(),
        Value::Array(items) => {
            let mut out = format!("*{}\r\n", items.len()).into_bytes();
            for item in items {
                out.extend(serialize(item));
            }
            out
        }
    }
}

pub async fn parse<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Value> {
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line).await?;
    let line = strip_crlf(&line)?;

    match line.first() {
        Some(b'+') => {
            let s = String::from_utf8(line[1..].to_vec())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            Ok(Value::SimpleString(s))
        }
        Some(b'-') => {
            let s = String::from_utf8(line[1..].to_vec())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            Ok(Value::Error(s))
        }
        Some(b':') => {
            let n = parse_integer(&line[1..])?;
            Ok(Value::Integer(n))
        }
        Some(b'$') => {
            let len = parse_integer(&line[1..])?;
            if len == -1 {
                return Ok(Value::Null);
            }
            if len < 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "negative bulk length"));
            }
            let len = len as usize;
            let mut buf = vec![0u8; len + 2];
            reader.read_exact(&mut buf).await?;
            if !buf.ends_with(b"\r\n") {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "missing bulk string CRLF"));
            }
            buf.truncate(len);
            Ok(Value::BulkString(buf))
        }
        Some(b'*') => {
            let count = parse_integer(&line[1..])?;
            if count == -1 {
                return Ok(Value::Null);
            }
            if count < 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "negative array length"));
            }
            let mut items = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let element = Box::pin(parse(reader)).await?;
                items.push(element);
            }
            Ok(Value::Array(items))
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "unknown RESP type prefix")),
    }
}

fn strip_crlf(line: &[u8]) -> io::Result<&[u8]> {
    if line.ends_with(b"\r\n") {
        Ok(&line[..line.len() - 2])
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "line missing CRLF terminator",
        ))
    }
}

fn parse_integer(bytes: &[u8]) -> io::Result<i64> {
    let s = std::str::from_utf8(bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    s.parse::<i64>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
