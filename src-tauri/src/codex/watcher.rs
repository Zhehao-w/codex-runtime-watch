use super::rollout::{adapt, Correlator};
use crate::db::Database;
use anyhow::Result;
use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};
pub fn scan_file(db: &Database, path: &Path, c: &mut Correlator) -> Result<usize> {
    Ok(scan_file_observations(db, path, c)?.len())
}
pub fn scan_file_observations(
    db: &Database,
    path: &Path,
    c: &mut Correlator,
) -> Result<Vec<crate::Observation>> {
    let key = path.to_string_lossy();
    let scope = format!(
        "rollout:{:016x}",
        key.bytes()
            .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b))
                .wrapping_mul(0x100000001b3))
    );
    let mut offset = db.offset(&key)?;
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    if offset > len {
        offset = 0
    }
    f.seek(SeekFrom::Start(offset))?;
    let mut r = BufReader::new(f);
    let mut observations = Vec::new();
    loop {
        let start = offset;
        let mut line = String::new();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        if !line.ends_with('\n') {
            offset = start;
            break;
        }
        offset += n as u64;
        if let Ok(v) = serde_json::from_str(&line) {
            if let Some(e) = adapt(&v) {
                if let Some(o) = c.push_scoped(&scope, e) {
                    let existed =
                        db.contains_turn(o.session_id.as_deref(), o.turn_id.as_deref(), &o.kind)?;
                    db.insert(&o)?;
                    if !existed {
                        observations.push(o)
                    }
                }
            }
        }
    }
    db.set_offset(&key, offset, None)?;
    Ok(observations)
}
