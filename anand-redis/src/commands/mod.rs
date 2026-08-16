use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::resp::Value;

pub mod generic;
pub mod list;
pub mod string;

/// Keys are typed in Redis, so every command that reads a value has to decide
/// what to do when it finds the wrong kind — see `CmdError::WrongType`.
pub enum Data {
    String(Vec<u8>),
    List(VecDeque<Vec<u8>>),
}

pub struct Entry {
    pub value: Data,
    pub expires_at: Option<Instant>,
}

pub type Db = Arc<RwLock<HashMap<Vec<u8>, Entry>>>;

/// Commands return protocol values on success and a `CmdError` on failure;
/// `dispatch` converts the error into a `Value::Error` at a single boundary.
pub type CmdResult = Result<Value, CmdError>;

#[derive(Debug)]
pub enum CmdError {
    WrongArity(&'static str),
    InvalidExpire(&'static str),
    Syntax(String),
    UnknownOption(String),
    UnknownCommand(String),
    Protocol(&'static str),
    NotAnInteger,
    Overflow,
    WrongType,
}

impl fmt::Display for CmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CmdError::WrongArity(cmd) => {
                write!(f, "ERR wrong number of arguments for '{cmd}' command")
            }
            CmdError::InvalidExpire(cmd) => write!(f, "ERR invalid expire time in '{cmd}' command"),
            CmdError::Syntax(msg) => write!(f, "ERR syntax error: {msg}"),
            CmdError::UnknownOption(opt) => write!(f, "ERR unknown option '{opt}'"),
            CmdError::UnknownCommand(cmd) => write!(f, "ERR unknown command '{cmd}'"),
            CmdError::Protocol(msg) => write!(f, "ERR {msg}"),
            CmdError::NotAnInteger => write!(f, "ERR value is not an integer or out of range"),
            CmdError::Overflow => write!(f, "ERR increment or decrement would overflow"),
            CmdError::WrongType => write!(
                f,
                "WRONGTYPE Operation against a key holding the wrong kind of value"
            ),
        }
    }
}

pub fn dispatch(value: Value, db: &Db) -> Value {
    run(value, db).unwrap_or_else(|err| Value::Error(err.to_string()))
}

fn run(value: Value, db: &Db) -> CmdResult {
    let Value::Array(parts) = value else {
        return Err(CmdError::Protocol("expected array command"));
    };

    // Clients always send commands as arrays of bulk strings, so unwrapping them
    // once here keeps every handler working on plain bytes.
    let args: Vec<Vec<u8>> = parts
        .into_iter()
        .map(|part| match part {
            Value::BulkString(bytes) => Ok(bytes),
            _ => Err(CmdError::Protocol(
                "command must be an array of bulk strings",
            )),
        })
        .collect::<Result<_, _>>()?;

    let Some((name, args)) = args.split_first() else {
        return Err(CmdError::Protocol("empty command array"));
    };

    match name.to_ascii_uppercase().as_slice() {
        b"PING" => generic::ping(args),
        b"ECHO" => generic::echo(args),
        b"DEL" => generic::del(args, db),
        b"EXISTS" => generic::exists(args, db),
        b"GET" => string::get(args, db),
        b"SET" => string::set(args, db),
        b"INCR" => string::incr_by(args, db, 1),
        b"DECR" => string::incr_by(args, db, -1),
        b"LPUSH" => list::push(args, db, list::Side::Left),
        b"RPUSH" => list::push(args, db, list::Side::Right),
        b"LRANGE" => list::lrange(args, db),
        other => Err(CmdError::UnknownCommand(
            String::from_utf8_lossy(other).into_owned(),
        )),
    }
}

/// `args` excludes the command name. `max` of `None` means "no upper bound".
pub(crate) fn arity(
    args: &[Vec<u8>],
    min: usize,
    max: Option<usize>,
    name: &'static str,
) -> Result<(), CmdError> {
    if args.len() < min || max.is_some_and(|max| args.len() > max) {
        return Err(CmdError::WrongArity(name));
    }
    Ok(())
}

/// Read paths treat an expired entry as absent without evicting it — the sweeper
/// in `server::run` reclaims it within 100 ms, which keeps GET and EXISTS on a
/// read lock instead of forcing a write lock just to delete.
pub(crate) fn is_expired(entry: &Entry) -> bool {
    entry
        .expires_at
        .is_some_and(|deadline| Instant::now() > deadline)
}

/// Write paths do evict, so a read-modify-write command starts from a clean slate
/// rather than resurrecting a key that should already be gone.
pub(crate) fn evict_if_expired(store: &mut HashMap<Vec<u8>, Entry>, key: &[u8]) {
    if store.get(key).is_some_and(is_expired) {
        store.remove(key);
    }
}
