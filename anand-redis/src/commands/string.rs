use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{CmdError, CmdResult, Db, Entry, arity, is_expired};
use crate::resp::Value;

pub fn get(args: &[Vec<u8>], db: &Db) -> CmdResult {
    arity(args, 1, Some(1), "get")?;
    let store = db.read().unwrap();
    match store.get(&args[0]) {
        Some(entry) if !is_expired(entry) => Ok(Value::BulkString(entry.value.clone())),
        _ => Ok(Value::Null),
    }
}

pub fn set(args: &[Vec<u8>], db: &Db) -> CmdResult {
    arity(args, 2, None, "set")?;
    let entry = Entry {
        value: args[1].clone(),
        expires_at: parse_expiry(&args[2..])?,
    };
    db.write().unwrap().insert(args[0].clone(), entry);
    Ok(Value::SimpleString("OK".to_string()))
}

fn parse_expiry(options: &[Vec<u8>]) -> Result<Option<Instant>, CmdError> {
    let mut expires_at = None;
    let mut i = 0;

    while i < options.len() {
        let flag = options[i].to_ascii_uppercase();
        match flag.as_slice() {
            b"EX" | b"PX" | b"EXAT" | b"PXAT" => {
                let Some(raw) = options.get(i + 1) else {
                    return Err(CmdError::Syntax(format!(
                        "{} requires an argument",
                        String::from_utf8_lossy(&flag)
                    )));
                };
                let amount: u64 = std::str::from_utf8(raw)
                    .ok()
                    .and_then(|text| text.parse().ok())
                    .filter(|amount| *amount > 0)
                    .ok_or(CmdError::InvalidExpire("set"))?;

                expires_at = Some(match flag.as_slice() {
                    b"EX" => Instant::now() + Duration::from_secs(amount),
                    b"PX" => Instant::now() + Duration::from_millis(amount),
                    b"EXAT" => unix_secs_to_instant(amount),
                    _ => unix_millis_to_instant(amount),
                });
                i += 2;
            }
            _ => {
                return Err(CmdError::UnknownOption(
                    String::from_utf8_lossy(&flag).into_owned(),
                ));
            }
        }
    }

    Ok(expires_at)
}

fn unix_secs_to_instant(unix_secs: u64) -> Instant {
    instant_from(UNIX_EPOCH + Duration::from_secs(unix_secs))
}

fn unix_millis_to_instant(unix_ms: u64) -> Instant {
    instant_from(UNIX_EPOCH + Duration::from_millis(unix_ms))
}

/// Expiries are stored as monotonic `Instant`s, so an absolute wall-clock target
/// has to be rebased against the current time. Targets already in the past
/// collapse to now, meaning the key expires immediately.
fn instant_from(target: SystemTime) -> Instant {
    let now = Instant::now();
    match target.duration_since(SystemTime::now()) {
        Ok(remaining) => now + remaining,
        Err(_) => now,
    }
}
