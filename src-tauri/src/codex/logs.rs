//! Optional `logs_2.sqlite` support is deliberately conservative: only structured explicit model
//! events are accepted. Unknown Codex log schemas are ignored rather than guessed.
use super::{models::Evidence, rollout};
use rusqlite::Connection;
use std::path::Path;
pub fn inspect(path: &Path) -> Vec<Evidence> {
    let Ok(c) = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return vec![];
    };
    let Ok(mut s)=c.prepare("SELECT message FROM logs WHERE message LIKE '%model/rerouted%' ORDER BY rowid DESC LIMIT 100") else{return vec![]};
    s.query_map([], |r| r.get::<_, String>(0))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|x| serde_json::from_str(&x).ok())
        .filter_map(|v| rollout::adapt(&v))
        .collect()
}
