use super::rollout::{adapt, Correlator};
use crate::db::Database;
use anyhow::Result;
use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};
pub fn scan_file(db: &Database, path: &Path, c: &mut Correlator) -> Result<usize> {
    let key = path.to_string_lossy();
    let mut offset = db.offset(&key)?;
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    if offset > len {
        offset = 0
    }
    f.seek(SeekFrom::Start(offset))?;
    let mut r = BufReader::new(f);
    let mut count = 0;
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
                if let Some(o) = c.push(e) {
                    db.insert(&o)?;
                    count += 1
                }
            }
        }
    }
    db.set_offset(&key, offset, None)?;
    Ok(count)
}
