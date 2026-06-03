use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::resp::Value;

pub struct Entry {
    pub value: Vec<u8>,
    pub expires_at: Option<Instant>,
}

pub type Db = Arc<RwLock<HashMap<Vec<u8>, Entry>>>;

pub fn dispatch(value: Value, db: &Db) -> Value {
    let Value::Array(mut parts) = value else {
        return Value::Error("ERR expected array command".to_string());
    };

    if parts.is_empty() {
        return Value::Error("ERR empty command array".to_string());
    }

    let Value::BulkString(cmd_bytes) = parts.remove(0) else {
        return Value::Error("ERR command name must be a bulk string".to_string());
    };

    let cmd = cmd_bytes.to_ascii_uppercase();

    match cmd.as_slice() {
        b"PING" => match parts.len() {
            0 => Value::SimpleString("PONG".to_string()),
            _ => parts.remove(0),
        },

        b"ECHO" => {
            if parts.is_empty() {
                Value::Error("ERR wrong number of arguments for 'echo' command".to_string())
            } else {
                parts.remove(0)
            }
        }

        b"SET" => {
            if parts.len() < 2 {
                return Value::Error(
                    "ERR wrong number of arguments for 'set' command".to_string(),
                );
            }
            let Value::BulkString(key) = parts.remove(0) else {
                return Value::Error("ERR key must be a bulk string".to_string());
            };
            let Value::BulkString(val) = parts.remove(0) else {
                return Value::Error("ERR value must be a bulk string".to_string());
            };

            let mut expiry: Option<Instant> = None;
            let mut i = 0;
            while i < parts.len() {
                let Value::BulkString(ref flag_bytes) = parts[i] else {
                    return Value::Error("ERR invalid option".to_string());
                };
                let flag = flag_bytes.to_ascii_uppercase();
                match flag.as_slice() {
                    b"EX" | b"PX" | b"EXAT" | b"PXAT" => {
                        if i + 1 >= parts.len() {
                            return Value::Error(format!(
                                "ERR syntax error: {} requires an argument",
                                String::from_utf8_lossy(&flag)
                            ));
                        }
                        let Value::BulkString(ref n_bytes) = parts[i + 1] else {
                            return Value::Error("ERR expiry must be an integer".to_string());
                        };
                        let n_str = match std::str::from_utf8(n_bytes) {
                            Ok(s) => s,
                            Err(_) => return Value::Error("ERR expiry must be an integer".to_string()),
                        };
                        let n: u64 = match n_str.parse() {
                            Ok(v) if v > 0 => v,
                            _ => return Value::Error("ERR invalid expire time in 'set' command".to_string()),
                        };
                        expiry = Some(match flag.as_slice() {
                            b"EX"   => Instant::now() + Duration::from_secs(n),
                            b"PX"   => Instant::now() + Duration::from_millis(n),
                            b"EXAT" => unix_secs_to_instant(n),
                            b"PXAT" => unix_millis_to_instant(n),
                            _ => unreachable!(),
                        });
                        i += 2;
                    }
                    _ => {
                        return Value::Error(format!(
                            "ERR unknown option '{}'",
                            String::from_utf8_lossy(&flag)
                        ));
                    }
                }
            }

            db.write().unwrap().insert(key, Entry { value: val, expires_at: expiry });
            Value::SimpleString("OK".to_string())
        }

        b"GET" => {
            if parts.is_empty() {
                return Value::Error(
                    "ERR wrong number of arguments for 'get' command".to_string(),
                );
            }
            let Value::BulkString(key) = parts.remove(0) else {
                return Value::Error("ERR key must be a bulk string".to_string());
            };

            let expired = {
                let store = db.read().unwrap();
                match store.get(&key) {
                    Some(Entry { expires_at: Some(exp), .. }) if Instant::now() > *exp => true,
                    Some(_) => false,
                    None => return Value::Null,
                }
            };

            if expired {
                db.write().unwrap().remove(&key);
                return Value::Null;
            }

            match db.read().unwrap().get(&key) {
                Some(entry) => Value::BulkString(entry.value.clone()),
                None => Value::Null,
            }
        }

        b"DEL" => {
            if parts.is_empty() {
                return Value::Error(
                    "ERR wrong number of arguments for 'del' command".to_string(),
                );
            }
            let keys: Vec<Vec<u8>> = parts
                .into_iter()
                .filter_map(|v| {
                    if let Value::BulkString(k) = v { Some(k) } else { None }
                })
                .collect();
            let mut store = db.write().unwrap();
            let deleted = keys.iter().filter(|k| store.remove(*k).is_some()).count();
            Value::Integer(deleted as i64)
        }

        _ => {
            let name = String::from_utf8_lossy(&cmd).into_owned();
            Value::Error(format!("ERR unknown command '{name}'"))
        }
    }
}

fn unix_secs_to_instant(unix_secs: u64) -> Instant {
    let target = UNIX_EPOCH + Duration::from_secs(unix_secs);
    let now_sys = SystemTime::now();
    let now_inst = Instant::now();
    match target.duration_since(now_sys) {
        Ok(d) => now_inst + d,
        Err(_) => now_inst, // already in the past — expires immediately
    }
}

fn unix_millis_to_instant(unix_ms: u64) -> Instant {
    let target = UNIX_EPOCH + Duration::from_millis(unix_ms);
    let now_sys = SystemTime::now();
    let now_inst = Instant::now();
    match target.duration_since(now_sys) {
        Ok(d) => now_inst + d,
        Err(_) => now_inst,
    }
}
