use crate::Observation;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Database(pub Connection);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationChange {
    New,
    Updated { notify: bool },
    Unchanged,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?
        }
        let c = Connection::open(path)?;
        c.busy_timeout(std::time::Duration::from_secs(3))?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA user_version=1; CREATE TABLE IF NOT EXISTS observations(id INTEGER PRIMARY KEY,time TEXT NOT NULL,type TEXT NOT NULL,session_id TEXT,turn_id TEXT,selected_model TEXT,selected_effort TEXT,runtime_model TEXT,runtime_effort TEXT,provider_model TEXT,evidence TEXT NOT NULL,details TEXT); CREATE UNIQUE INDEX IF NOT EXISTS observations_turn ON observations(session_id,turn_id,type) WHERE turn_id IS NOT NULL; CREATE INDEX IF NOT EXISTS observations_time ON observations(time DESC); CREATE TABLE IF NOT EXISTS scan_state(source TEXT PRIMARY KEY,offset INTEGER NOT NULL DEFAULT 0,metadata TEXT);")?;
        Ok(Self(c))
    }
    pub fn insert(&self, o: &Observation) -> Result<i64> {
        self.0.execute("INSERT INTO observations(time,type,session_id,turn_id,selected_model,selected_effort,runtime_model,runtime_effort,provider_model,evidence,details) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(session_id,turn_id,type) WHERE turn_id IS NOT NULL DO UPDATE SET selected_model=COALESCE(excluded.selected_model,selected_model),selected_effort=COALESCE(excluded.selected_effort,selected_effort),runtime_model=COALESCE(excluded.runtime_model,runtime_model),runtime_effort=COALESCE(excluded.runtime_effort,runtime_effort),provider_model=COALESCE(excluded.provider_model,provider_model),evidence=excluded.evidence,details=excluded.details",params![o.time,o.kind,o.session_id,o.turn_id,o.selected_model,o.selected_effort,o.runtime_model,o.runtime_effort,o.provider_model,o.evidence,o.details])?;
        Ok(self.0.last_insert_rowid())
    }
    fn turn(
        &self,
        session: Option<&str>,
        turn: Option<&str>,
        kind: &str,
    ) -> Result<Option<Observation>> {
        let Some(turn) = turn else { return Ok(None) };
        self.0.query_row("SELECT id,time,type,session_id,turn_id,selected_model,selected_effort,runtime_model,runtime_effort,provider_model,evidence,details FROM observations WHERE session_id IS ? AND turn_id=? AND type=? LIMIT 1", params![session,turn,kind], |r| Ok(Observation { id:r.get(0)?,time:r.get(1)?,kind:r.get(2)?,session_id:r.get(3)?,turn_id:r.get(4)?,selected_model:r.get(5)?,selected_effort:r.get(6)?,runtime_model:r.get(7)?,runtime_effort:r.get(8)?,provider_model:r.get(9)?,evidence:r.get(10)?,details:r.get(11)? })).optional().map_err(Into::into)
    }
    pub fn upsert_turn(&self, o: &Observation) -> Result<(ObservationChange, Observation)> {
        let before = self.turn(o.session_id.as_deref(), o.turn_id.as_deref(), &o.kind)?;
        self.insert(o)?;
        let Some(after) = self.turn(o.session_id.as_deref(), o.turn_id.as_deref(), &o.kind)? else {
            anyhow::bail!("turn upsert did not produce an observation")
        };
        let change = match before {
            None => ObservationChange::New,
            Some(before) if before == after => ObservationChange::Unchanged,
            Some(before) => ObservationChange::Updated {
                notify: before.provider_model != after.provider_model
                    && after.provider_model.is_some(),
            },
        };
        Ok((change, after))
    }
    pub fn offset(&self, p: &str) -> Result<u64> {
        Ok(self
            .0
            .query_row("SELECT offset FROM scan_state WHERE source=?", [p], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
            .unwrap_or(0) as u64)
    }
    pub fn scan_state(&self, p: &str) -> Result<(u64, Option<String>)> {
        Ok(self
            .0
            .query_row(
                "SELECT offset,metadata FROM scan_state WHERE source=?",
                [p],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((0, None)))
    }
    pub fn set_offset(&self, p: &str, n: u64, meta: Option<&str>) -> Result<()> {
        self.0.execute("INSERT INTO scan_state(source,offset,metadata) VALUES(?,?,?) ON CONFLICT(source) DO UPDATE SET offset=excluded.offset,metadata=excluded.metadata",params![p,n as i64,meta])?;
        Ok(())
    }
    pub fn history(&self, filter: &str, limit: u32, offset: u32) -> Result<Vec<Observation>> {
        let predicate = match filter {
            "runtime" => "type='Runtime'",
            "probes" => "type='Probe'",
            "mismatches" => "(provider_model IS NOT NULL AND (selected_model IS NULL OR provider_model != selected_model)) OR (provider_model IS NULL AND selected_model IS NOT NULL AND runtime_model IS NOT NULL AND (selected_model != runtime_model OR (selected_effort IS NOT NULL AND runtime_effort IS NOT NULL AND selected_effort != runtime_effort)))",
            _ => "1=1",
        };
        let sql = format!("SELECT id,time,type,session_id,turn_id,selected_model,selected_effort,runtime_model,runtime_effort,provider_model,evidence,details FROM observations WHERE {predicate} ORDER BY time DESC,id DESC LIMIT ? OFFSET ?");
        let mut s = self.0.prepare(&sql)?;
        let rows = s
            .query_map(params![limit.min(500), offset], |r| {
                Ok(Observation {
                    id: r.get(0)?,
                    time: r.get(1)?,
                    kind: r.get(2)?,
                    session_id: r.get(3)?,
                    turn_id: r.get(4)?,
                    selected_model: r.get(5)?,
                    selected_effort: r.get(6)?,
                    runtime_model: r.get(7)?,
                    runtime_effort: r.get(8)?,
                    provider_model: r.get(9)?,
                    evidence: r.get(10)?,
                    details: r.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    pub fn current_runtime(&self) -> Result<Option<Observation>> {
        Ok(self.history("runtime", 1, 0)?.into_iter().next())
    }
    pub fn delete(&self, id: i64) -> Result<()> {
        self.0
            .execute("DELETE FROM observations WHERE id=?", [id])?;
        Ok(())
    }
    pub fn clear(&self) -> Result<()> {
        self.0.execute("DELETE FROM observations", [])?;
        Ok(())
    }
}
