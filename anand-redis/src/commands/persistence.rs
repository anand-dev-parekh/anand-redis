use std::collections::{HashMap, VecDeque};
use std::io;

use tokio::io::BufReader;

use super::{
    CmdResult, Data, Db, Entry, arity, is_expired, unix_millis_from, unix_millis_to_instant,
};
use crate::resp::{self, Value};

const DUMP_PATH: &str = "dump.rdb";
const TMP_PATH: &str = "dump.rdb.tmp";

/// The dump is RESP rather than real RDB, so a key's type has to be recorded
/// explicitly — the wire format has no notion of a `Data` variant.
const STRING_TAG: &[u8] = b"string";
const LIST_TAG: &[u8] = b"list";

/// Blocks the connection while it writes, exactly like the real `SAVE`. Keeping
/// it synchronous is what lets `dispatch` stay synchronous.
pub fn save(args: &[Vec<u8>], db: &Db) -> CmdResult {
    arity(args, 0, Some(0), "save")?;

    // `Entry` isn't `Clone`, so the RESP tree is built while the read lock is
    // held. The guard drops before the write, so a slow disk can't block writers.
    let snapshot = {
        let store = db.read().unwrap();
        resp::serialize(&encode(&store))
    };

    // Write beside the dump and rename, so an interrupted save leaves the
    // previous dump intact rather than a half-written one.
    std::fs::write(TMP_PATH, &snapshot)?;
    if let Err(err) = std::fs::rename(TMP_PATH, DUMP_PATH) {
        let _ = std::fs::remove_file(TMP_PATH);
        return Err(err.into());
    }

    Ok(Value::SimpleString("OK".to_string()))
}

/// Startup path. A missing dump is the ordinary first-run case and yields an
/// empty store; anything else is an error for the caller to report.
pub async fn load() -> io::Result<HashMap<Vec<u8>, Entry>> {
    let file = match tokio::fs::File::open(DUMP_PATH).await {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(err) => return Err(err),
    };

    // One top-level array, so a single `parse` consumes the whole file. Reading
    // entry-by-entry couldn't tell a clean EOF from a truncated dump, since
    // `parse` reports both as `InvalidData`.
    let mut reader = BufReader::new(file);
    decode(resp::parse(&mut reader).await?)
}

/// `[tag, key, expires_at_ms, value...]` per entry, all wrapped in one array.
fn encode(store: &HashMap<Vec<u8>, Entry>) -> Value {
    let entries = store
        .iter()
        .filter(|(_, entry)| !is_expired(entry))
        .map(|(key, entry)| {
            let (tag, values) = match &entry.value {
                Data::String(bytes) => (STRING_TAG, vec![Value::BulkString(bytes.clone())]),
                // A VecDeque can't be sliced, so the list is walked head to tail.
                Data::List(list) => (
                    LIST_TAG,
                    list.iter()
                        .map(|item| Value::BulkString(item.clone()))
                        .collect(),
                ),
            };

            // Instants are meaningless to the next process, so expiries go out as
            // Unix milliseconds.
            let expires_at = entry.expires_at.map_or(Value::Null, |deadline| {
                Value::Integer(unix_millis_from(deadline) as i64)
            });

            let mut fields = vec![
                Value::BulkString(tag.to_vec()),
                Value::BulkString(key.clone()),
                expires_at,
            ];
            fields.extend(values);
            Value::Array(fields)
        })
        .collect();

    Value::Array(entries)
}

fn decode(value: Value) -> io::Result<HashMap<Vec<u8>, Entry>> {
    let Value::Array(entries) = value else {
        return Err(corrupt("dump is not an array"));
    };

    let mut store = HashMap::with_capacity(entries.len());

    for entry in entries {
        let Value::Array(fields) = entry else {
            return Err(corrupt("dump entry is not an array"));
        };
        let [tag, key, expires_at, values @ ..] = fields.as_slice() else {
            return Err(corrupt("dump entry is missing its tag, key or expiry"));
        };
        let (Value::BulkString(tag), Value::BulkString(key)) = (tag, key) else {
            return Err(corrupt("dump tag and key must be bulk strings"));
        };

        let expires_at = match expires_at {
            Value::Null => None,
            // A timestamp that has already passed collapses to now, so a key
            // whose TTL elapsed while the server was down reloads as expired and
            // the sweeper reclaims it — the same rule EXAT follows.
            Value::Integer(unix_ms) => Some(unix_millis_to_instant(
                u64::try_from(*unix_ms).map_err(|_| corrupt("dump expiry is negative"))?,
            )),
            _ => return Err(corrupt("dump expiry is neither an integer nor null")),
        };

        let mut items: Vec<Vec<u8>> = Vec::with_capacity(values.len());
        for value in values {
            let Value::BulkString(bytes) = value else {
                return Err(corrupt("dump value is not a bulk string"));
            };
            items.push(bytes.clone());
        }

        let value = match tag.as_slice() {
            STRING_TAG if items.len() == 1 => Data::String(items.remove(0)),
            STRING_TAG => return Err(corrupt("a string entry must hold exactly one value")),
            LIST_TAG => Data::List(VecDeque::from(items)),
            _ => return Err(corrupt("unknown entry type tag")),
        };

        store.insert(key.clone(), Entry { value, expires_at });
    }

    Ok(store)
}

fn corrupt(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}
