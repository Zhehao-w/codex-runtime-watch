use super::rollout::{adapt, Correlator, PersistedScope};
use crate::{
    db::{Database, ObservationChange},
    Observation,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

const FINGERPRINT_LIMIT: u64 = 4096;

#[derive(Serialize, Deserialize)]
struct ScanMetadata {
    version: u8,
    prefix_len: u64,
    prefix_hash: String,
    scope: PersistedScope,
}

fn fingerprint(file: &mut File, prefix_len: u64) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = vec![0; prefix_len as usize];
    file.read_exact(&mut bytes)?;
    let hash = bytes.into_iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    Ok(format!("{hash:016x}"))
}

pub fn scan_file(db: &Database, path: &Path, c: &mut Correlator) -> Result<usize> {
    Ok(scan_file_observations(db, path, c)?.len())
}
pub fn scan_file_observations(
    db: &Database,
    path: &Path,
    c: &mut Correlator,
) -> Result<Vec<ScanObservation>> {
    let key = path.to_string_lossy();
    let scope = format!(
        "rollout:{:016x}",
        key.bytes()
            .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b))
                .wrapping_mul(0x100000001b3))
    );
    let (mut offset, metadata) = db.scan_state(&key)?;
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let persisted = metadata
        .as_deref()
        .and_then(|value| serde_json::from_str::<ScanMetadata>(value).ok());
    let replacement = if offset > len {
        true
    } else if offset > 0 {
        match persisted.as_ref() {
            Some(state) if state.version == 1 && state.prefix_len <= len => {
                fingerprint(&mut file, state.prefix_len)? != state.prefix_hash
            }
            _ => true,
        }
    } else {
        false
    };
    if replacement {
        offset = 0;
        c.reset_scope(&scope);
    } else if let Some(state) = persisted {
        c.restore_scope(&scope, &serde_json::to_string(&state.scope)?);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::new(file);
    let mut observations = Vec::new();
    loop {
        let start = offset;
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        if !line.ends_with('\n') {
            offset = start;
            break;
        }
        offset += n as u64;
        if let Ok(value) = serde_json::from_str(&line) {
            if let Some(evidence) = adapt(&value) {
                if let Some(observation) = c.push_scoped(&scope, evidence) {
                    let (change, observation) = db.upsert_turn(&observation)?;
                    match change {
                        ObservationChange::New => observations.push(ScanObservation {
                            observation,
                            notify: true,
                        }),
                        ObservationChange::Updated { notify } => {
                            observations.push(ScanObservation {
                                observation,
                                notify,
                            })
                        }
                        ObservationChange::Unchanged => {}
                    }
                }
            }
        }
    }
    let prefix_len = offset.min(FINGERPRINT_LIMIT);
    let mut file = reader.into_inner();
    let metadata = ScanMetadata {
        version: 1,
        prefix_len,
        prefix_hash: fingerprint(&mut file, prefix_len)?,
        scope: serde_json::from_str(&c.persist_scope(&scope))?,
    };
    db.set_offset(&key, offset, Some(&serde_json::to_string(&metadata)?))?;
    // scan_state owns the durable parser context. Releasing this scope keeps
    // memory bounded by active scans rather than lifetime rollout-file count.
    c.reset_scope(&scope);
    Ok(observations)
}

#[derive(Debug)]
pub struct ScanObservation {
    pub observation: Observation,
    pub notify: bool,
}
