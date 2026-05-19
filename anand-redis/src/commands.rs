use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::resp::Value;

pub type Db = Arc<RwLock<HashMap<Vec<u8>, Vec<u8>>>>;

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
            db.write().unwrap().insert(key, val);
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
            match db.read().unwrap().get(&key) {
                Some(val) => Value::BulkString(val.clone()),
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
