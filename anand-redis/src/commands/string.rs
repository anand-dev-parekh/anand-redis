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

pub fn incr_by(args: &[Vec<u8>], db: &Db, delta: i64) -> CmdResult {
    let name = if delta > 0 { "incr" } else { "decr" };
    arity(args, 1, Some(1), name)?;

    let mut store = db.write().unwrap();
    let key = &args[0];

    if store.get(key).is_some_and(is_expired) {
        store.remove(key);
    }

    let entry = store.entry(key.clone()).or_insert_with(|| Entry {
        value: b"0".to_vec(),
        expires_at: None,
    });

    let current: i64 = std::str::from_utf8(&entry.value)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or(CmdError::NotAnInteger)?;

    let next = current.checked_add(delta).ok_or(CmdError::Overflow)?;

    // Assign to `value` only — replacing the whole Entry would wipe the TTL.
    entry.value = next.to_string().into_bytes();

    Ok(Value::Integer(next))
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
