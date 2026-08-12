use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{Result, VaultError, key_header};

const DATABASE_FILE: &str = "vault.db";
const KEY_HEADER_FILE: &str = "vault-key.json";
const MAX_HEADER_BYTES: u64 = 16 * 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_CHANNEL_ID_BYTES: usize = 128;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_MLS_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const MAX_MLS_OUTBOX_BYTES: usize = 384 * 1024;
const MAX_SEARCH_BYTES: usize = 256;
const LIST_LIMIT: usize = 1000;
const SEARCH_LIMIT: usize = 100;
const OUTBOX_LIMIT: usize = 1000;
const SCHEMA_VERSION: u32 = 4;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultState {
    Uninitialized,
    Locked,
    Unlocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VaultStatus {
    pub state: VaultState,
    pub schema_version: Option<u32>,
    pub message_count: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NewVaultMessage {
    pub id: String,
    pub channel_id: String,
    pub body: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VaultMessage {
    pub id: String,
    pub channel_id: String,
    pub body: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultOutboxItem {
    pub device_id: String,
    pub event_id: String,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct Vault {
    paths: VaultPaths,
    connection: Option<Connection>,
}

impl Vault {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            paths: VaultPaths::new(root.into()),
            connection: None,
        }
    }

    pub fn status(&self) -> Result<VaultStatus> {
        let disk_state = self.disk_state()?;
        if self.connection.is_some() && disk_state != VaultState::Locked {
            return Err(VaultError::CorruptState);
        }

        if let Some(connection) = self.connection.as_ref() {
            let message_count =
                connection.query_row("SELECT COUNT(*) FROM local_messages", [], |row| {
                    row.get::<_, u64>(0)
                })?;
            let schema_version =
                connection.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))?;

            return Ok(VaultStatus {
                state: VaultState::Unlocked,
                schema_version: Some(schema_version),
                message_count: Some(message_count),
            });
        }

        Ok(VaultStatus {
            state: disk_state,
            schema_version: None,
            message_count: None,
        })
    }

    pub fn initialize(&mut self, passphrase: &str) -> Result<VaultStatus> {
        if self.connection.is_some() || self.disk_state()? != VaultState::Uninitialized {
            return Err(VaultError::AlreadyInitialized);
        }

        let (header_bytes, data_key) = key_header::create(passphrase)?;
        let parent = self.paths.root.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let initialization_root = create_initialization_directory(parent, &self.paths.root)?;
        let initialization_paths = VaultPaths::new(initialization_root.clone());

        let initialization_result = (|| {
            create_database(&initialization_paths.database, data_key.as_ref())?;
            write_private_file(&initialization_paths.key_header, &header_bytes)?;
            sync_directory(&initialization_root)?;
            fs::rename(&initialization_root, &self.paths.root)?;
            sync_directory(parent)?;
            Ok(())
        })();

        if let Err(error) = initialization_result {
            let _ = fs::remove_dir_all(&initialization_root);
            return Err(error);
        }

        self.connection = Some(open_database(&self.paths.database, data_key.as_ref())?);
        self.status()
    }

    pub fn unlock(&mut self, passphrase: &str) -> Result<VaultStatus> {
        if self.connection.is_some() {
            return self.status();
        }
        if self.disk_state()? != VaultState::Locked {
            return Err(VaultError::NotInitialized);
        }

        let header_metadata = fs::metadata(&self.paths.key_header)?;
        if !header_metadata.is_file() || header_metadata.len() > MAX_HEADER_BYTES {
            return Err(VaultError::CorruptState);
        }
        let header_bytes = fs::read(&self.paths.key_header)?;
        let data_key = key_header::unlock(&header_bytes, passphrase)?;
        self.connection = Some(open_database(&self.paths.database, data_key.as_ref())?);
        self.status()
    }

    pub fn lock(&mut self) -> Result<VaultStatus> {
        if let Some(connection) = self.connection.take() {
            close_connection(connection)?;
        }
        self.status()
    }

    pub fn store_message(&self, message: &NewVaultMessage) -> Result<VaultMessage> {
        validate_message(message)?;
        insert_message(self.connection()?, message)
    }

    pub fn list_messages(&self, channel_id: &str) -> Result<Vec<VaultMessage>> {
        validate_channel_id(channel_id)?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, channel_id, body, created_at_ms
             FROM local_messages
             WHERE channel_id = ?1
             ORDER BY created_at_ms ASC, id ASC
             LIMIT ?2",
        )?;
        let messages = statement
            .query_map(params![channel_id, LIST_LIMIT], map_message)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    pub fn search_messages(&self, channel_id: &str, query: &str) -> Result<Vec<VaultMessage>> {
        validate_channel_id(channel_id)?;
        if query.len() > MAX_SEARCH_BYTES {
            return Err(VaultError::InvalidInput("Der Suchbegriff ist zu lang."));
        }
        let fts_query = create_fts_query(query)?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT messages.id, messages.channel_id, messages.body, messages.created_at_ms
             FROM local_messages_fts
             JOIN local_messages AS messages ON messages.rowid = local_messages_fts.rowid
             WHERE local_messages_fts MATCH ?1 AND messages.channel_id = ?2
             ORDER BY messages.created_at_ms ASC, messages.id ASC
             LIMIT ?3",
        )?;
        let messages = statement
            .query_map(params![fts_query, channel_id, SEARCH_LIMIT], map_message)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    pub fn local_device_id(&self) -> Result<Uuid> {
        let connection = self.connection()?;
        let stored: Option<String> = connection
            .query_row(
                "SELECT device_id FROM local_device WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(stored) = stored {
            return Uuid::parse_str(&stored).map_err(|_| VaultError::CorruptState);
        }

        let device_id = Uuid::new_v4();
        connection.execute(
            "INSERT INTO local_device (singleton, device_id) VALUES (1, ?1)",
            params![device_id.to_string()],
        )?;
        Ok(device_id)
    }

    pub fn store_mls_client_snapshot(&self, device_id: &str, snapshot: &[u8]) -> Result<()> {
        validate_identifier(device_id, MAX_ID_BYTES, "Die MLS-Geraete-ID ist ungueltig.")?;
        validate_mls_snapshot(snapshot)?;

        self.connection()?.execute(
            "INSERT INTO mls_client_snapshots (device_id, snapshot)
             VALUES (?1, ?2)
             ON CONFLICT(device_id) DO UPDATE SET snapshot = excluded.snapshot",
            params![device_id, snapshot],
        )?;
        Ok(())
    }

    pub fn store_mls_snapshot_and_outbox(
        &self,
        device_id: &str,
        snapshot: &[u8],
        event_id: &str,
        payload: &[u8],
    ) -> Result<()> {
        self.store_mls_transaction(device_id, snapshot, event_id, payload, None)?;
        Ok(())
    }

    pub fn store_message_and_mls_snapshot_and_outbox(
        &self,
        message: &NewVaultMessage,
        device_id: &str,
        snapshot: &[u8],
        event_id: &str,
        payload: &[u8],
    ) -> Result<VaultMessage> {
        validate_message(message)?;
        self.store_mls_transaction(device_id, snapshot, event_id, payload, Some(message))?
            .ok_or(VaultError::CorruptState)
    }

    fn store_mls_transaction(
        &self,
        device_id: &str,
        snapshot: &[u8],
        event_id: &str,
        payload: &[u8],
        message: Option<&NewVaultMessage>,
    ) -> Result<Option<VaultMessage>> {
        validate_identifier(device_id, MAX_ID_BYTES, "Die MLS-Geraete-ID ist ungueltig.")?;
        validate_identifier(event_id, MAX_ID_BYTES, "Die MLS-Event-ID ist ungueltig.")?;
        validate_mls_snapshot(snapshot)?;
        validate_mls_outbox_payload(payload)?;

        let transaction = self.connection()?.unchecked_transaction()?;
        let existing: Option<(String, Vec<u8>)> = transaction
            .query_row(
                "SELECT device_id, payload FROM mls_outbox WHERE event_id = ?1",
                params![event_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((existing_device_id, existing_payload)) = existing {
            if existing_device_id != device_id || existing_payload != payload {
                return Err(VaultError::OutboxConflict);
            }
            let existing_snapshot = transaction.query_row(
                "SELECT snapshot FROM mls_client_snapshots WHERE device_id = ?1",
                params![device_id],
                |row| row.get::<_, Vec<u8>>(0),
            )?;
            if existing_snapshot != snapshot {
                return Err(VaultError::OutboxConflict);
            }
            let stored_message = match message {
                Some(message) => {
                    let stored = transaction
                        .query_row(
                            "SELECT id, channel_id, body, created_at_ms
                             FROM local_messages WHERE id = ?1",
                            params![message.id],
                            map_message,
                        )
                        .optional()?;
                    let Some(stored) = stored else {
                        return Err(VaultError::OutboxConflict);
                    };
                    if stored.id != message.id
                        || stored.channel_id != message.channel_id
                        || stored.body != message.body
                        || stored.created_at_ms != message.created_at_ms
                    {
                        return Err(VaultError::OutboxConflict);
                    }
                    Some(stored)
                }
                None => None,
            };
            transaction.commit()?;
            return Ok(stored_message);
        }

        let stored_message = message
            .map(|message| insert_message(&transaction, message))
            .transpose()?;
        transaction.execute(
            "INSERT INTO mls_client_snapshots (device_id, snapshot)
             VALUES (?1, ?2)
             ON CONFLICT(device_id) DO UPDATE SET snapshot = excluded.snapshot",
            params![device_id, snapshot],
        )?;
        transaction.execute(
            "INSERT INTO mls_outbox (event_id, device_id, payload)
             VALUES (?1, ?2, ?3)",
            params![event_id, device_id, payload],
        )?;
        transaction.commit()?;
        Ok(stored_message)
    }

    pub fn list_mls_outbox(&self, device_id: &str) -> Result<Vec<VaultOutboxItem>> {
        validate_identifier(device_id, MAX_ID_BYTES, "Die MLS-Geraete-ID ist ungueltig.")?;
        let mut statement = self.connection()?.prepare(
            "SELECT device_id, event_id, payload
             FROM mls_outbox
             WHERE device_id = ?1
             ORDER BY rowid ASC
             LIMIT ?2",
        )?;
        let items = statement
            .query_map(params![device_id, OUTBOX_LIMIT], |row| {
                Ok(VaultOutboxItem {
                    device_id: row.get(0)?,
                    event_id: row.get(1)?,
                    payload: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        validate_mls_outbox_items(&items)?;
        Ok(items)
    }

    pub fn list_all_mls_outbox(&self) -> Result<Vec<VaultOutboxItem>> {
        let mut statement = self.connection()?.prepare(
            "SELECT device_id, event_id, payload
             FROM mls_outbox
             ORDER BY rowid ASC
             LIMIT ?1",
        )?;
        let items = statement
            .query_map(params![OUTBOX_LIMIT], |row| {
                Ok(VaultOutboxItem {
                    device_id: row.get(0)?,
                    event_id: row.get(1)?,
                    payload: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        validate_mls_outbox_items(&items)?;
        Ok(items)
    }

    pub fn acknowledge_mls_outbox(&self, device_id: &str, event_id: &str) -> Result<bool> {
        validate_identifier(device_id, MAX_ID_BYTES, "Die MLS-Geraete-ID ist ungueltig.")?;
        validate_identifier(event_id, MAX_ID_BYTES, "Die MLS-Event-ID ist ungueltig.")?;
        Ok(self.connection()?.execute(
            "DELETE FROM mls_outbox WHERE device_id = ?1 AND event_id = ?2",
            params![device_id, event_id],
        )? == 1)
    }

    pub fn load_mls_client_snapshot(&self, device_id: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        validate_identifier(device_id, MAX_ID_BYTES, "Die MLS-Geraete-ID ist ungueltig.")?;
        let snapshot: Option<Vec<u8>> = self
            .connection()?
            .query_row(
                "SELECT snapshot FROM mls_client_snapshots WHERE device_id = ?1",
                params![device_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(VaultError::from)?;
        match snapshot {
            Some(snapshot) if snapshot.is_empty() || snapshot.len() > MAX_MLS_SNAPSHOT_BYTES => {
                Err(VaultError::CorruptState)
            }
            Some(snapshot) => Ok(Some(Zeroizing::new(snapshot))),
            None => Ok(None),
        }
    }

    fn connection(&self) -> Result<&Connection> {
        self.connection.as_ref().ok_or(VaultError::Locked)
    }

    fn disk_state(&self) -> Result<VaultState> {
        if !self.paths.root.exists() {
            return Ok(VaultState::Uninitialized);
        }
        if !self.paths.root.is_dir() {
            return Err(VaultError::CorruptState);
        }

        let database_exists = self.paths.database.is_file();
        let header_exists = self.paths.key_header.is_file();
        if database_exists && header_exists {
            Ok(VaultState::Locked)
        } else {
            Err(VaultError::CorruptState)
        }
    }
}

#[derive(Debug)]
struct VaultPaths {
    root: PathBuf,
    database: PathBuf,
    key_header: PathBuf,
}

impl VaultPaths {
    fn new(root: PathBuf) -> Self {
        Self {
            database: root.join(DATABASE_FILE),
            key_header: root.join(KEY_HEADER_FILE),
            root,
        }
    }
}

fn create_database(path: &Path, data_key: &[u8]) -> Result<()> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
    let connection = Connection::open_with_flags(path, flags)?;
    apply_raw_key(&connection, data_key)?;
    verify_sqlcipher(&connection)?;
    configure_connection(&connection, false)?;
    connection.execute_batch(
        "CREATE TABLE local_messages (
            id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 128),
            channel_id TEXT NOT NULL CHECK(length(channel_id) BETWEEN 1 AND 128),
            body TEXT NOT NULL CHECK(length(body) BETWEEN 1 AND 65536),
            created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
        ) STRICT;

        CREATE INDEX local_messages_channel_time
            ON local_messages(channel_id, created_at_ms, id);

        CREATE VIRTUAL TABLE local_messages_fts USING fts5(
            body,
            content='local_messages',
            content_rowid='rowid',
            tokenize='unicode61 remove_diacritics 2'
        );

        CREATE TRIGGER local_messages_after_insert AFTER INSERT ON local_messages BEGIN
            INSERT INTO local_messages_fts(rowid, body) VALUES (new.rowid, new.body);
        END;

        CREATE TRIGGER local_messages_after_delete AFTER DELETE ON local_messages BEGIN
            INSERT INTO local_messages_fts(local_messages_fts, rowid, body)
            VALUES ('delete', old.rowid, old.body);
        END;

        CREATE TRIGGER local_messages_after_update AFTER UPDATE ON local_messages BEGIN
            INSERT INTO local_messages_fts(local_messages_fts, rowid, body)
            VALUES ('delete', old.rowid, old.body);
            INSERT INTO local_messages_fts(rowid, body) VALUES (new.rowid, new.body);
        END;

        CREATE TABLE mls_client_snapshots (
            device_id TEXT PRIMARY KEY NOT NULL CHECK(length(device_id) BETWEEN 1 AND 128),
            snapshot BLOB NOT NULL CHECK(length(snapshot) BETWEEN 1 AND 16777216)
        ) STRICT;

        CREATE TABLE mls_outbox (
            event_id TEXT PRIMARY KEY NOT NULL CHECK(length(event_id) BETWEEN 1 AND 128),
            device_id TEXT NOT NULL CHECK(length(device_id) BETWEEN 1 AND 128),
            payload BLOB NOT NULL CHECK(length(payload) BETWEEN 1 AND 393216),
            FOREIGN KEY(device_id) REFERENCES mls_client_snapshots(device_id) ON DELETE CASCADE
        ) STRICT;

        CREATE INDEX mls_outbox_device ON mls_outbox(device_id);

        CREATE TABLE local_device (
            singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
            device_id TEXT NOT NULL UNIQUE CHECK(length(device_id) = 36)
        ) STRICT;

        PRAGMA user_version = 4;",
    )?;
    close_connection(connection)?;
    restrict_file_permissions(path)?;
    Ok(())
}

fn open_database(path: &Path, data_key: &[u8]) -> Result<Connection> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    apply_raw_key(&connection, data_key)?;
    verify_sqlcipher(&connection)?;
    configure_connection(&connection, true)?;
    let schema_version =
        connection.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))?;
    match schema_version {
        SCHEMA_VERSION => {}
        1 => {
            migrate_v1_to_v2(&mut connection)?;
            migrate_v2_to_v3(&mut connection)?;
            migrate_v3_to_v4(&mut connection)?;
        }
        2 => {
            migrate_v2_to_v3(&mut connection)?;
            migrate_v3_to_v4(&mut connection)?;
        }
        3 => migrate_v3_to_v4(&mut connection)?,
        other => return Err(VaultError::UnsupportedVersion(other)),
    }
    Ok(connection)
}

fn migrate_v3_to_v4(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE local_device (
            singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
            device_id TEXT NOT NULL UNIQUE CHECK(length(device_id) = 36)
        ) STRICT;
        PRAGMA user_version = 4;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn migrate_v2_to_v3(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE mls_outbox (
            event_id TEXT PRIMARY KEY NOT NULL CHECK(length(event_id) BETWEEN 1 AND 128),
            device_id TEXT NOT NULL CHECK(length(device_id) BETWEEN 1 AND 128),
            payload BLOB NOT NULL CHECK(length(payload) BETWEEN 1 AND 393216),
            FOREIGN KEY(device_id) REFERENCES mls_client_snapshots(device_id) ON DELETE CASCADE
        ) STRICT;
        CREATE INDEX mls_outbox_device ON mls_outbox(device_id);
        PRAGMA user_version = 3;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn migrate_v1_to_v2(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE mls_client_snapshots (
            device_id TEXT PRIMARY KEY NOT NULL CHECK(length(device_id) BETWEEN 1 AND 128),
            snapshot BLOB NOT NULL CHECK(length(snapshot) BETWEEN 1 AND 16777216)
        ) STRICT;
        PRAGMA user_version = 2;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn apply_raw_key(connection: &Connection, data_key: &[u8]) -> Result<()> {
    if data_key.len() != 32 {
        return Err(VaultError::Crypto);
    }
    let encoded_key = hex::encode(data_key);
    connection.execute_batch(&format!("PRAGMA key = \"x'{encoded_key}'\";"))?;
    Ok(())
}

fn verify_sqlcipher(connection: &Connection) -> Result<()> {
    let version = connection
        .query_row("PRAGMA cipher_version", [], |row| row.get::<_, String>(0))
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => VaultError::SqlCipherUnavailable,
            other => VaultError::Database(other),
        })?;
    if version.is_empty() {
        return Err(VaultError::SqlCipherUnavailable);
    }
    connection.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| {
        row.get::<_, u64>(0)
    })?;
    Ok(())
}

fn configure_connection(connection: &Connection, use_wal: bool) -> Result<()> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA cipher_memory_security = ON;
         PRAGMA foreign_keys = ON;
         PRAGMA secure_delete = ON;
         PRAGMA temp_store = MEMORY;",
    )?;
    let journal_mode = if use_wal { "WAL" } else { "DELETE" };
    connection.query_row(
        &format!("PRAGMA journal_mode = {journal_mode}"),
        [],
        |row| row.get::<_, String>(0),
    )?;
    Ok(())
}

fn close_connection(connection: Connection) -> Result<()> {
    let maintenance_result = connection.execute_batch(
        "PRAGMA wal_checkpoint(TRUNCATE);
         PRAGMA optimize;",
    );
    let close_result = connection.close().map_err(|(_, error)| error);
    maintenance_result?;
    close_result?;
    Ok(())
}

fn create_initialization_directory(parent: &Path, vault_root: &Path) -> Result<PathBuf> {
    let vault_name = vault_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("vault");

    for _ in 0..4 {
        let mut random_suffix = [0_u8; 8];
        getrandom::fill(&mut random_suffix).map_err(|_| VaultError::Crypto)?;
        let path = parent.join(format!(".{vault_name}-init-{}", hex::encode(random_suffix)));
        match fs::create_dir(&path) {
            Ok(()) => {
                restrict_directory_permissions(&path)?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(VaultError::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique vault initialization directory",
    )))
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    restrict_file_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn validate_message(message: &NewVaultMessage) -> Result<()> {
    validate_identifier(
        &message.id,
        MAX_ID_BYTES,
        "Die Nachrichten-ID ist ungueltig.",
    )?;
    validate_channel_id(&message.channel_id)?;
    if message.body.trim().is_empty() || message.body.len() > MAX_MESSAGE_BYTES {
        return Err(VaultError::InvalidInput(
            "Der Nachrichteninhalt ist ungueltig.",
        ));
    }
    if message.created_at_ms < 0 {
        return Err(VaultError::InvalidInput(
            "Der Nachrichtenzeitpunkt ist ungueltig.",
        ));
    }
    Ok(())
}

fn insert_message(connection: &Connection, message: &NewVaultMessage) -> Result<VaultMessage> {
    connection.execute(
        "INSERT INTO local_messages (id, channel_id, body, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
        params![
            message.id,
            message.channel_id,
            message.body,
            message.created_at_ms
        ],
    )?;
    Ok(VaultMessage {
        id: message.id.clone(),
        channel_id: message.channel_id.clone(),
        body: message.body.clone(),
        created_at_ms: message.created_at_ms,
    })
}

fn validate_mls_snapshot(snapshot: &[u8]) -> Result<()> {
    if snapshot.is_empty() || snapshot.len() > MAX_MLS_SNAPSHOT_BYTES {
        return Err(VaultError::InvalidInput(
            "Der MLS-Zustand ist leer oder zu gross.",
        ));
    }
    Ok(())
}

fn validate_mls_outbox_payload(payload: &[u8]) -> Result<()> {
    if payload.is_empty() || payload.len() > MAX_MLS_OUTBOX_BYTES {
        return Err(VaultError::InvalidInput(
            "Der MLS-Sendeauftrag ist leer oder zu gross.",
        ));
    }
    Ok(())
}

fn validate_mls_outbox_items(items: &[VaultOutboxItem]) -> Result<()> {
    if items.iter().any(|item| {
        validate_identifier(
            &item.device_id,
            MAX_ID_BYTES,
            "Die MLS-Geraete-ID ist ungueltig.",
        )
        .is_err()
            || validate_identifier(
                &item.event_id,
                MAX_ID_BYTES,
                "Die MLS-Event-ID ist ungueltig.",
            )
            .is_err()
            || validate_mls_outbox_payload(&item.payload).is_err()
    }) {
        return Err(VaultError::CorruptState);
    }
    Ok(())
}

fn validate_channel_id(channel_id: &str) -> Result<()> {
    validate_identifier(
        channel_id,
        MAX_CHANNEL_ID_BYTES,
        "Die Kanal-ID ist ungueltig.",
    )
}

fn validate_identifier(value: &str, max_bytes: usize, message: &'static str) -> Result<()> {
    if value.is_empty()
        || value.len() > max_bytes
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(VaultError::InvalidInput(message));
    }
    Ok(())
}

fn create_fts_query(query: &str) -> Result<String> {
    let terms = query
        .split_whitespace()
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Err(VaultError::InvalidInput("Der Suchbegriff ist leer."));
    }
    Ok(terms.join(" AND "))
}

fn map_message(row: &Row<'_>) -> rusqlite::Result<VaultMessage> {
    Ok(VaultMessage {
        id: row.get(0)?,
        channel_id: row.get(1)?,
        body: row.get(2)?,
        created_at_ms: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use rusqlite::Connection;
    use serde_json::Value;
    use tempfile::tempdir;

    use super::{KEY_HEADER_FILE, NewVaultMessage, Vault, VaultError, VaultState};

    const PASSPHRASE: &str = "correct horse battery staple";
    const CANARY: &str = "SINGULARIS-CANARY-only-inside-the-unlocked-vault";
    const MLS_CANARY: &[u8] = b"MLS-PRIVATE-STATE-CANARY";

    #[test]
    fn encrypted_vault_survives_lock_and_restart_without_plaintext_canary() {
        let temporary_directory = tempdir().expect("temporary directory");
        let vault_root = temporary_directory.path().join("vault-v1");
        let mut vault = Vault::new(&vault_root);

        assert_eq!(
            vault.status().expect("initial status").state,
            VaultState::Uninitialized
        );
        let initialized = vault.initialize(PASSPHRASE).expect("initialize vault");
        assert_eq!(initialized.state, VaultState::Unlocked);
        assert_private_permissions(&vault_root);

        let message = NewVaultMessage {
            id: "local-message-1".to_owned(),
            channel_id: "entwicklung".to_owned(),
            body: CANARY.to_owned(),
            created_at_ms: 1_786_300_000_000,
        };
        vault.store_message(&message).expect("store message");
        vault
            .store_mls_client_snapshot("01989550-e7f8-7000-8000-000000000001", MLS_CANARY)
            .expect("store MLS snapshot");
        assert_eq!(
            vault
                .load_mls_client_snapshot("01989550-e7f8-7000-8000-000000000001")
                .expect("load MLS snapshot")
                .as_ref()
                .map(|snapshot| snapshot.as_slice()),
            Some(MLS_CANARY)
        );
        assert_eq!(
            vault.list_messages("entwicklung").expect("list messages")[0].body,
            CANARY
        );
        assert_eq!(
            vault
                .search_messages("entwicklung", "unlocked vault")
                .expect("search messages")
                .len(),
            1
        );

        assert_eq!(vault.lock().expect("lock vault").state, VaultState::Locked);
        assert!(matches!(
            vault.list_messages("entwicklung"),
            Err(VaultError::Locked)
        ));
        assert_directory_does_not_contain(&vault_root, CANARY.as_bytes());
        assert_directory_does_not_contain(&vault_root, MLS_CANARY);

        let unkeyed = Connection::open(vault_root.join("vault.db")).expect("open encrypted file");
        let unkeyed_read = unkeyed.query_row("SELECT COUNT(*) FROM local_messages", [], |row| {
            row.get::<_, u64>(0)
        });
        assert!(
            unkeyed_read.is_err(),
            "database must reject an unkeyed read"
        );

        let mut reopened = Vault::new(&vault_root);
        assert!(matches!(
            reopened.unlock("this is the wrong passphrase"),
            Err(VaultError::AuthenticationFailed)
        ));
        reopened.unlock(PASSPHRASE).expect("unlock after restart");
        assert_eq!(
            reopened
                .list_messages("entwicklung")
                .expect("list after restart")[0]
                .body,
            CANARY
        );
        assert_eq!(
            reopened
                .load_mls_client_snapshot("01989550-e7f8-7000-8000-000000000001")
                .expect("load MLS snapshot after restart")
                .as_ref()
                .map(|snapshot| snapshot.as_slice()),
            Some(MLS_CANARY)
        );
    }

    #[test]
    fn modified_wrapped_key_is_rejected_as_authentication_failure() {
        let temporary_directory = tempdir().expect("temporary directory");
        let vault_root = temporary_directory.path().join("vault-v1");
        let mut vault = Vault::new(&vault_root);
        vault.initialize(PASSPHRASE).expect("initialize vault");
        vault.lock().expect("lock vault");

        let header_path = vault_root.join(KEY_HEADER_FILE);
        let mut header: Value =
            serde_json::from_slice(&fs::read(&header_path).expect("read key header"))
                .expect("parse key header");
        let wrapped_key = header["wrapped_key"].as_str().expect("wrapped key string");
        let replacement = if wrapped_key.starts_with('A') {
            'B'
        } else {
            'A'
        };
        let mut modified_key = wrapped_key.to_owned();
        modified_key.replace_range(..1, &replacement.to_string());
        header["wrapped_key"] = Value::String(modified_key);
        fs::write(
            &header_path,
            serde_json::to_vec_pretty(&header).expect("serialize modified header"),
        )
        .expect("write modified header");

        let mut reopened = Vault::new(&vault_root);
        assert!(matches!(
            reopened.unlock(PASSPHRASE),
            Err(VaultError::AuthenticationFailed)
        ));
    }

    #[test]
    fn schema_one_vault_migrates_before_storing_mls_state() {
        let temporary_directory = tempdir().expect("temporary directory");
        let vault_root = temporary_directory.path().join("vault-v1");
        let mut vault = Vault::new(&vault_root);
        vault.initialize(PASSPHRASE).expect("initialize vault");
        vault
            .connection()
            .expect("unlocked connection")
            .execute_batch(
                "DROP TABLE mls_outbox;
                 DROP TABLE mls_client_snapshots;
                 DROP TABLE local_device;
                 PRAGMA user_version = 1;",
            )
            .expect("downgrade schema fixture");
        vault.lock().expect("lock schema-one vault");

        let mut reopened = Vault::new(&vault_root);
        let status = reopened.unlock(PASSPHRASE).expect("migrate schema");
        assert_eq!(status.schema_version, Some(4));
        reopened
            .store_mls_client_snapshot("01989550-e7f8-7000-8000-000000000002", MLS_CANARY)
            .expect("store MLS state after migration");
        assert_eq!(
            reopened
                .load_mls_client_snapshot("01989550-e7f8-7000-8000-000000000002")
                .expect("load migrated MLS state")
                .as_ref()
                .map(|snapshot| snapshot.as_slice()),
            Some(MLS_CANARY)
        );
    }

    #[test]
    fn schema_two_vault_preserves_mls_state_and_adds_the_outbox() {
        let temporary_directory = tempdir().expect("temporary directory");
        let vault_root = temporary_directory.path().join("vault-v2");
        let mut vault = Vault::new(&vault_root);
        vault.initialize(PASSPHRASE).expect("initialize vault");
        let device_id = "01989550-e7f8-7000-8000-000000000005";
        let event_id = "01989550-e7f8-7000-8000-000000000006";
        vault
            .store_mls_client_snapshot(device_id, MLS_CANARY)
            .expect("store schema-two MLS snapshot");
        vault
            .connection()
            .expect("unlocked connection")
            .execute_batch(
                "DROP TABLE mls_outbox;
                 DROP TABLE local_device;
                 PRAGMA user_version = 2;",
            )
            .expect("create schema-two fixture");
        vault.lock().expect("lock schema-two vault");

        let mut reopened = Vault::new(&vault_root);
        let status = reopened.unlock(PASSPHRASE).expect("migrate schema two");
        assert_eq!(status.schema_version, Some(4));
        assert_eq!(
            reopened
                .load_mls_client_snapshot(device_id)
                .unwrap()
                .unwrap()
                .as_slice(),
            MLS_CANARY
        );
        reopened
            .store_mls_snapshot_and_outbox(
                device_id,
                MLS_CANARY,
                event_id,
                b"first schema-three outbox payload",
            )
            .expect("use migrated outbox");
        assert_eq!(reopened.list_mls_outbox(device_id).unwrap().len(), 1);
    }

    #[test]
    fn local_device_identity_is_created_once_and_survives_restart() {
        let temporary_directory = tempdir().expect("temporary directory");
        let vault_root = temporary_directory.path().join("vault-v1");
        let mut vault = Vault::new(&vault_root);
        vault.initialize(PASSPHRASE).expect("initialize vault");

        let device_id = vault.local_device_id().expect("create local device ID");
        assert_eq!(vault.local_device_id().unwrap(), device_id);
        vault.lock().expect("lock vault");

        let mut reopened = Vault::new(&vault_root);
        let status = reopened.unlock(PASSPHRASE).expect("unlock after restart");
        assert_eq!(status.schema_version, Some(4));
        assert_eq!(reopened.local_device_id().unwrap(), device_id);
    }

    #[test]
    fn mls_snapshot_and_outbox_are_atomic_idempotent_and_restart_safe() {
        let temporary_directory = tempdir().expect("temporary directory");
        let vault_root = temporary_directory.path().join("vault-v1");
        let mut vault = Vault::new(&vault_root);
        vault.initialize(PASSPHRASE).expect("initialize vault");
        let device_id = "01989550-e7f8-7000-8000-000000000003";
        let event_id = "01989550-e7f8-7000-8000-000000000004";
        let payload = b"opaque canonical SubmitEvent";

        vault
            .store_mls_snapshot_and_outbox(device_id, MLS_CANARY, event_id, payload)
            .expect("commit snapshot and outbox");
        vault
            .store_mls_snapshot_and_outbox(device_id, MLS_CANARY, event_id, payload)
            .expect("repeat identical commit");
        assert_eq!(vault.list_mls_outbox(device_id).unwrap().len(), 1);

        assert!(matches!(
            vault.store_mls_snapshot_and_outbox(
                device_id,
                b"different snapshot",
                event_id,
                payload
            ),
            Err(VaultError::OutboxConflict)
        ));
        assert!(matches!(
            vault.store_mls_snapshot_and_outbox(
                device_id,
                b"snapshot that must roll back",
                event_id,
                b"different payload"
            ),
            Err(VaultError::OutboxConflict)
        ));
        assert_eq!(
            vault
                .load_mls_client_snapshot(device_id)
                .unwrap()
                .unwrap()
                .as_slice(),
            MLS_CANARY
        );

        vault.lock().expect("lock vault");
        drop(vault);
        let mut reopened = Vault::new(&vault_root);
        reopened.unlock(PASSPHRASE).expect("unlock after restart");
        assert_eq!(
            reopened.list_mls_outbox(device_id).unwrap(),
            vec![super::VaultOutboxItem {
                device_id: device_id.to_owned(),
                event_id: event_id.to_owned(),
                payload: payload.to_vec(),
            }]
        );
        assert_eq!(
            reopened.list_all_mls_outbox().unwrap(),
            reopened.list_mls_outbox(device_id).unwrap()
        );
        assert!(
            reopened
                .acknowledge_mls_outbox(device_id, event_id)
                .unwrap()
        );
        assert!(
            !reopened
                .acknowledge_mls_outbox(device_id, event_id)
                .unwrap()
        );
        assert!(reopened.list_mls_outbox(device_id).unwrap().is_empty());
    }

    #[test]
    fn message_snapshot_and_outbox_roll_back_together() {
        let temporary_directory = tempdir().expect("temporary directory");
        let mut vault = Vault::new(temporary_directory.path().join("vault-v1"));
        vault.initialize(PASSPHRASE).expect("initialize vault");
        let message = NewVaultMessage {
            id: "01989550-e7f8-7000-8000-000000000021".to_owned(),
            channel_id: "briefing".to_owned(),
            body: "atomic local and encrypted message".to_owned(),
            created_at_ms: 1_754_700_000_000,
        };
        vault
            .store_message(&message)
            .expect("seed duplicate message");
        let device_id = "01989550-e7f8-7000-8000-000000000022";
        let event_id = "01989550-e7f8-7000-8000-000000000023";

        assert!(
            vault
                .store_message_and_mls_snapshot_and_outbox(
                    &message,
                    device_id,
                    MLS_CANARY,
                    event_id,
                    b"opaque canonical SubmitEvent",
                )
                .is_err()
        );
        assert!(vault.list_mls_outbox(device_id).unwrap().is_empty());
        assert!(vault.load_mls_client_snapshot(device_id).unwrap().is_none());
        assert_eq!(vault.list_messages("briefing").unwrap().len(), 1);
    }

    #[test]
    fn weak_passphrases_and_partial_vaults_are_rejected() {
        let temporary_directory = tempdir().expect("temporary directory");
        let vault_root = temporary_directory.path().join("vault-v1");
        let mut vault = Vault::new(&vault_root);
        assert!(matches!(
            vault.initialize("too short"),
            Err(VaultError::WeakPassphrase)
        ));

        fs::create_dir(&vault_root).expect("create partial vault root");
        fs::write(vault_root.join("vault.db"), b"partial").expect("write partial database");
        assert!(matches!(vault.status(), Err(VaultError::CorruptState)));
        assert!(matches!(
            vault.initialize(PASSPHRASE),
            Err(VaultError::CorruptState)
        ));
    }

    fn assert_directory_does_not_contain(path: &Path, needle: &[u8]) {
        for entry in fs::read_dir(path).expect("read vault directory") {
            let entry = entry.expect("vault directory entry");
            let entry_path = entry.path();
            if entry_path.is_dir() {
                assert_directory_does_not_contain(&entry_path, needle);
                continue;
            }

            let bytes = fs::read(&entry_path).expect("read vault file");
            assert!(
                !bytes.windows(needle.len()).any(|window| window == needle),
                "plaintext canary leaked into {}",
                entry_path.display()
            );
        }
    }

    #[cfg(unix)]
    fn assert_private_permissions(vault_root: &Path) {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(vault_root)
                .expect("vault root metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for file_name in ["vault.db", KEY_HEADER_FILE] {
            assert_eq!(
                fs::metadata(vault_root.join(file_name))
                    .expect("vault file metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(not(unix))]
    fn assert_private_permissions(_vault_root: &Path) {}
}
