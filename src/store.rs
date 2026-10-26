use crate::model::{state_dir, Persisted};
use anyhow::Result;
use rusqlite::{params, Connection};

pub struct Store {
    db: Connection,
}
impl Store {
    pub fn open() -> Result<Self> {
        let dir = state_dir();
        std::fs::create_dir_all(&dir)?;
        crate::platform::private_permissions(&dir, 0o700)?;
        let db = Connection::open(dir.join("state.sqlite"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=3000; CREATE TABLE IF NOT EXISTS state (id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL, payload TEXT NOT NULL);")?;
        Ok(Self { db })
    }
    pub fn load(&self) -> Result<Persisted> {
        let mut q = self.db.prepare("SELECT payload FROM state WHERE id=1")?;
        let mut rows = q.query([])?;
        Ok(if let Some(row) = rows.next()? {
            serde_json::from_str(&row.get::<_, String>(0)?)?
        } else {
            Persisted {
                sidebar: true,
                sidebar_width: 28,
                ..Default::default()
            }
        })
    }
    pub fn save(&self, data: &Persisted) -> Result<()> {
        self.db.execute("INSERT INTO state(id,version,payload) VALUES(1,1,?1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![serde_json::to_string(data)?])?;
        Ok(())
    }
}
