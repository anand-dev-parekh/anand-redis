use std::time::{Duration, Instant};

use super::{
    CmdError, CmdResult, Data, Db, Entry, arity, evict_if_expired, is_expired,
    unix_millis_to_instant, unix_secs_to_instant,
};
use crate::resp::Value;

pub fn get(args: &[Vec<u8>], db: &Db) -> CmdResult {
    arity(args, 1, Some(1), "get")?;
    let store = db.read().unwrap();
    match store.get(&args[0]) {
        Some(entry) if !is_expired(entry) => match &entry.value {
            Data::String(bytes) => Ok(Value::BulkString(bytes.clone())),
            Data::List(_) => Err(CmdError::WrongType),
        },
        _ => Ok(Value::Null),
    }
}

pub fn set(args: &[Vec<u8>], db: &Db) -> CmdResult {
    arity(args, 2, None, "set")?;

    let entry = Entry {
        value: Data::String(args[1].clone()),
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

    evict_if_expired(&mut store, key);

    let entry = store.entry(key.clone()).or_insert_with(|| Entry {
        value: Data::String(b"0".to_vec()),
        expires_at: None,
    });

    let current: i64 = match &entry.value {
        Data::String(bytes) => std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or(CmdError::NotAnInteger)?,
        Data::List(_) => return Err(CmdError::WrongType),
    };

    let next = current.checked_add(delta).ok_or(CmdError::Overflow)?;

    // Assign to `value` only — replacing the whole Entry would wipe the TTL.
    entry.value = Data::String(next.to_string().into_bytes());

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
