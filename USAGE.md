# Current State & Usage

A Redis-compatible TCP server written in Rust. It speaks the [RESP protocol](https://redis.io/docs/reference/protocol-spec/) so any standard Redis client works out of the box.

## Running

```bash
cd anand-redis
cargo run
# Listening on 127.0.0.1:6379
```

Connect with the Redis CLI or `redis-cli`:

```bash
redis-cli -p 6379
```

## Supported Commands

| Command | Usage | Returns |
|---------|-------|---------|
| `PING` | `PING` | `PONG` |
| `PING` | `PING <message>` | `<message>` echoed back |
| `ECHO` | `ECHO <message>` | `<message>` |
| `SET` | `SET <key> <value> [EX seconds\|PX milliseconds\|EXAT unix-time-seconds\|PXAT unix-time-milliseconds]` | `OK` |
| `GET` | `GET <key>` | value, or `nil` if missing |
| `DEL` | `DEL <key> [key ...]` | number of keys deleted |
| `EXISTS` | `EXISTS <key> [key ...]` | number of keys that exist (duplicates counted separately) |
| `INCR` | `INCR <key>` | the new value after adding 1 |
| `DECR` | `DECR <key>` | the new value after subtracting 1 |
| `LPUSH` | `LPUSH <key> <value> [value ...]` | list length after the push |
| `RPUSH` | `RPUSH <key> <value> [value ...]` | list length after the push |
| `LRANGE` | `LRANGE <key> <start> <stop>` | the elements in that inclusive range |
| `SAVE` | `SAVE` | `OK` once the dataset is on disk |

`INCR` and `DECR` treat a missing key as `0`, so `INCR` on a fresh key returns `1`. Any existing TTL
is preserved. Values are stored as strings, so `SET n 10` then `INCR n` leaves `n` holding `"11"`.
A value that isn't a valid `i64`, or arithmetic that would overflow one, returns an error.

`LPUSH` inserts each value at the head **in argument order**, so `LPUSH k a b c` leaves the list as
`[c, b, a]`; `RPUSH` appends, giving `[a, b, c]`. Both create the list if the key is missing.

`LRANGE` bounds are inclusive at both ends, and a negative index counts back from the tail — `-1` is
the last element, so `LRANGE k 0 -1` returns everything. Out-of-range bounds are clamped rather than
rejected, and a missing key yields an empty array rather than an error.

Keys are typed. Using a string command on a list (or the reverse) returns
`WRONGTYPE Operation against a key holding the wrong kind of value` — so `GET` and `INCR` reject a
list key, and `LPUSH`/`LRANGE` reject a string key. `SET` is the exception: it replaces a key of any
type, list included.

## Persistence

`SAVE` writes the whole dataset to `./dump.rdb`, relative to the directory the server was started
in, and the server loads that file back at startup. There is no automatic or background save, so
nothing reaches disk unless you ask for it.

`SAVE` is **synchronous**, meaning the client that issued it gets no reply until the data is on
disk. It does not stall the server: other connections keep being served for the whole write, and
reads (`GET`, `EXISTS`, `LRANGE`) are never blocked by a save at all. Writes pause only for the
brief in-memory encode at the start, not for the disk. Real Redis is single-threaded and does stall
during `SAVE`, so this is one place the behaviour deliberately differs.

The file isn't real RDB. It's the same RESP encoding the server speaks on the wire: one array of
`[type, key, expires-at-ms, value...]` entries, so you can read a dump with `cat -v`. TTLs survive a
restart because they're written as absolute Unix milliseconds; a key whose TTL elapsed while the
server was down does not come back. Already-expired keys are skipped at save time, and the write
goes to `dump.rdb.tmp` first and is renamed into place, so an interrupted save leaves the previous
dump intact.

If `dump.rdb` is missing, the server starts empty — that's the normal first run. If it exists but
can't be read or parsed, the server prints a warning to stderr and still starts, with an empty
dataset.

## Example Session

```
127.0.0.1:6379> PING
PONG
127.0.0.1:6379> SET name alice
OK
127.0.0.1:6379> GET name
"alice"
127.0.0.1:6379> DEL name
(integer) 1
127.0.0.1:6379> GET name
(nil)
127.0.0.1:6379> SET session token EX 60
OK
127.0.0.1:6379> SET job result PX 500
OK
127.0.0.1:6379> SET event data EXAT 1800000000
OK
127.0.0.1:6379> SET a 1
OK
127.0.0.1:6379> EXISTS a missing
(integer) 1
127.0.0.1:6379> EXISTS a a
(integer) 2
127.0.0.1:6379> INCR hits
(integer) 1
127.0.0.1:6379> INCR hits
(integer) 2
127.0.0.1:6379> DECR hits
(integer) 1
127.0.0.1:6379> GET hits
"1"
127.0.0.1:6379> SET word hello
OK
127.0.0.1:6379> INCR word
(error) ERR value is not an integer or out of range
127.0.0.1:6379> RPUSH letters a b c
(integer) 3
127.0.0.1:6379> LPUSH letters z
(integer) 4
127.0.0.1:6379> LRANGE letters 0 -1
1) "z"
2) "a"
3) "b"
4) "c"
127.0.0.1:6379> LRANGE letters -2 -1
1) "b"
2) "c"
127.0.0.1:6379> GET letters
(error) WRONGTYPE Operation against a key holding the wrong kind of value
127.0.0.1:6379> SAVE
OK
```

Restart the server after that `SAVE` and `GET name` still returns `"alice"`.

## Architecture

```
main.rs             — entry point, starts the async runtime
server.rs           — TCP listener, spawns a tokio task per client connection
resp.rs             — RESP serializer and async parser
commands/
  mod.rs            — dispatch, shared types, arity and expiry helpers
  generic.rs        — PING, ECHO, DEL, EXISTS
  string.rs         — GET, SET, INCR, DECR
  list.rs           — LPUSH, RPUSH, LRANGE
  persistence.rs    — SAVE, plus the dump encoder/decoder used at startup
```

**Store:** an in-memory `HashMap<Vec<u8>, Entry>` wrapped in `Arc<RwLock<...>>` so all client tasks share it safely. `Entry` holds an optional expiry `Instant` plus a value, which is either `Data::String(Vec<u8>)` or `Data::List(VecDeque<Vec<u8>>)` — `VecDeque` so `LPUSH` inserts at the head in O(1). Keys and values are bytes rather than `String`, so both are binary-safe. The store lives in memory and only reaches disk when you run `SAVE`.

**Persistence:** the dump reuses `resp::serialize` and `resp::parse` as the on-disk format rather than implementing RDB, so there's no second codec to maintain — `parse` is generic over `AsyncBufRead`, which makes a `tokio::fs::File` a valid source with no changes to `resp.rs`. `SAVE` builds the RESP tree while holding the **read** lock, drops the lock, and only then writes — so the disk never blocks anyone, and the only contention a save adds is writers waiting out that in-memory encode. The blocking write costs one tokio worker thread, not the runtime.

**Command layer:** `dispatch` unwraps the inbound array into `Vec<Vec<u8>>` once, then routes to a handler grouped by data type. Handlers return `Result<Value, CmdError>`, and `dispatch` converts an error into a RESP error at that single boundary. `dispatch` is deliberately synchronous — it uses `std::sync::RwLock`, and staying sync makes it impossible to hold a lock guard across an `.await`.

**Concurrency:** each client connection runs in its own `tokio::spawn` task. `GET` and `EXISTS` take a read lock and treat an expired key as absent without deleting it; `SET` and `DEL` take a write lock. A background task sweeps the store every 100 ms and removes expired keys (O(n) active expiry), so nothing needs to evict on the read path.

## Limitations (not yet implemented)

- No AOF, no `BGSAVE`, and no automatic save — persistence happens only when you run `SAVE`
- No pub/sub
- No transactions (`MULTI`/`EXEC`)
- No data types beyond plain strings (lists, sets, hashes, sorted sets)
