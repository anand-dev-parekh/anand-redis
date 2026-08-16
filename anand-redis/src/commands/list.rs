use std::collections::VecDeque;

use super::{CmdError, CmdResult, Data, Db, Entry, arity, evict_if_expired, is_expired};
use crate::resp::Value;

#[derive(Clone, Copy)]
pub enum Side {
    Left,
    Right,
}

pub fn push(args: &[Vec<u8>], db: &Db, side: Side) -> CmdResult {
    let name = match side {
        Side::Left => "lpush",
        Side::Right => "rpush",
    };
    arity(args, 2, None, name)?;

    let mut store = db.write().unwrap();
    let key = &args[0];
    evict_if_expired(&mut store, key);

    let entry = store.entry(key.clone()).or_insert_with(|| Entry {
        value: Data::List(VecDeque::new()),
        expires_at: None,
    });

    // Mutating through `value` leaves `expires_at` alone, so a push keeps any TTL.
    // The insert above only runs when the key is absent, and always creates a list,
    // so returning here can never leave an empty list behind.
    let Data::List(list) = &mut entry.value else {
        return Err(CmdError::WrongType);
    };

    for value in &args[1..] {
        match side {
            // Each value goes to the head in argument order, so LPUSH k a b c
            // leaves the list as [c, b, a].
            Side::Left => list.push_front(value.clone()),
            Side::Right => list.push_back(value.clone()),
        }
    }

    Ok(Value::Integer(list.len() as i64))
}

pub fn lrange(args: &[Vec<u8>], db: &Db) -> CmdResult {
    arity(args, 3, Some(3), "lrange")?;
    let start = parse_index(&args[1])?;
    let stop = parse_index(&args[2])?;

    let store = db.read().unwrap();
    let Some(entry) = store.get(&args[0]).filter(|entry| !is_expired(entry)) else {
        // A missing key is an empty list here — not nil, and not an error.
        return Ok(Value::Array(Vec::new()));
    };
    let Data::List(list) = &entry.value else {
        return Err(CmdError::WrongType);
    };

    // Redis ranges are inclusive at both ends, and a negative index counts back
    // from the tail: -1 is the last element.
    let len = list.len() as i64;
    let start = if start < 0 { start + len } else { start };
    let stop = if stop < 0 { stop + len } else { stop };
    let start = start.max(0);
    let stop = stop.min(len - 1);

    if len == 0 || start > stop {
        return Ok(Value::Array(Vec::new()));
    }

    // A VecDeque wraps around its buffer, so it can't be sliced — `range` walks it.
    let items = list
        .range(start as usize..=stop as usize)
        .map(|value| Value::BulkString(value.clone()))
        .collect();
    Ok(Value::Array(items))
}

fn parse_index(raw: &[u8]) -> Result<i64, CmdError> {
    std::str::from_utf8(raw)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or(CmdError::NotAnInteger)
}
