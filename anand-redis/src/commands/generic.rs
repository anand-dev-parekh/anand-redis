use super::{Db, CmdResult, arity, is_expired};
use crate::resp::Value;

pub fn ping(args: &[Vec<u8>]) -> CmdResult {
    arity(args, 0, Some(1), "ping")?;
    match args.first() {
        Some(message) => Ok(Value::BulkString(message.clone())),
        None => Ok(Value::SimpleString("PONG".to_string())),
    }
}

pub fn echo(args: &[Vec<u8>]) -> CmdResult {
    arity(args, 1, Some(1), "echo")?;
    Ok(Value::BulkString(args[0].clone()))
}

pub fn del(args: &[Vec<u8>], db: &Db) -> CmdResult {
    arity(args, 1, None, "del")?;
    let mut store = db.write().unwrap();
    let deleted = args
        .iter()
        .filter(|key| {
            store
                .remove(*key)
                .is_some_and(|entry| !is_expired(&entry))
        })
        .count();
    Ok(Value::Integer(deleted as i64))
}

pub fn exists(args: &[Vec<u8>], db: &Db) -> CmdResult {
    arity(args, 1, None, "exists")?;
    let store = db.read().unwrap();
    // Redis counts duplicates: EXISTS a a is 2.
    let found = args
        .iter()
        .filter(|key| store.get(*key).is_some_and(|entry| !is_expired(entry)))
        .count();
    Ok(Value::Integer(found as i64))
}
