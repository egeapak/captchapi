//! Configuration persisted in the database.
//!
//! Holds the `config_settings` table, which becomes a configuration layer at boot, and the
//! `config_generations` bookkeeping that lets a configuration which prevents startup roll back
//! on its own rather than wedging the service.
//!
//! Values are stored as raw strings in the same canonical form every other layer produces, so
//! parsing and validation stay in `Config::from_env_provider` and nowhere else. The table is
//! keyed by `Config` field name — what the admin API speaks — while a [`Layer`] is keyed by
//! canonical environment key, so [`ConfigStore::load`] translates between the two.

use crate::config::params::{by_field, Param, Persist};
use crate::config::sources::Layer;
use crate::error::{AppError, Result};
use chrono::Utc;
use sqlx::sqlite::SqlitePool;
use sqlx::Row;
use std::collections::BTreeMap;

/// Who wrote a stored setting, recorded for the audit trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrittenBy {
    AdminApi,
    Cli,
}

impl WrittenBy {
    fn label(self) -> &'static str {
        match self {
            WrittenBy::AdminApi => "admin-api",
            WrittenBy::Cli => "cli",
        }
    }
}

/// What the generation bookkeeping decided during startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootOutcome {
    /// Nothing pending: either no stored configuration has ever been written, or the newest
    /// generation is already confirmed.
    Clean { generation: Option<i64> },
    /// A generation is being tried for the first time. Confirm it once the process has proved
    /// it can serve.
    Trying { generation: i64 },
    /// The newest generation had already been attempted and the process did not survive, so
    /// the last confirmed settings were restored.
    RolledBack {
        generation: i64,
        restored_fields: usize,
    },
}

#[derive(Clone)]
pub struct ConfigStore {
    pool: SqlitePool,
}

impl ConfigStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Every stored setting, keyed by `Config` field name.
    ///
    /// This is the raw table, for `GET /config/stored`. Use [`Self::load`] to get something the
    /// configuration stack can consume.
    pub async fn all(&self) -> Result<BTreeMap<String, String>> {
        let rows = sqlx::query("SELECT field, value FROM config_settings ORDER BY field")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get::<String, _>("field"), r.get::<String, _>("value")))
            .collect())
    }

    /// Every stored row naming a field this service will actually honour.
    ///
    /// Rows naming a field that is unknown or `Persist::Never` are **dropped with a warning**
    /// rather than honoured. Nothing this service writes can produce such a row, so one can
    /// only arrive by hand-editing the database or by a downgrade — and honouring it would let
    /// an edit to a data file override a secret or the database location itself. Dropping is
    /// also why the load path cannot be a plain `SELECT *`.
    ///
    /// Both the configuration layer and the admin API's view are built from this, so the two
    /// can never disagree about which rows count. Reading raw rows for display was a real hole:
    /// every secret is `Persist::Never`, so a planted `master_api_key` row was dropped from the
    /// configuration and then echoed in the clear by `GET /config/stored` — the one surface in
    /// this codebase that did not redact.
    async fn honoured(&self) -> Result<Vec<(&'static Param, String)>> {
        let mut kept = Vec::new();
        for (field, value) in self.all().await? {
            match by_field(&field) {
                Some(param) if param.persist == Persist::Allowed => kept.push((param, value)),
                Some(param) => {
                    tracing::warn!(
                        "Ignoring stored `{}`: this setting is never read from the database",
                        param.field
                    );
                }
                None => {
                    tracing::warn!("Ignoring stored `{field}`: not a configuration field");
                }
            }
        }
        Ok(kept)
    }

    /// The stored settings as a configuration layer, keyed by canonical environment key.
    pub async fn load(&self) -> Result<Layer> {
        Ok(self
            .honoured()
            .await?
            .into_iter()
            .map(|(param, value)| (param.env.to_string(), value))
            .collect())
    }

    /// The stored settings as the admin API reports them, keyed by `Config` field name.
    ///
    /// Deliberately not [`Self::all`]: this is a response body, and it must describe the
    /// configuration the server is actually running, not the raw table.
    pub async fn visible(&self) -> Result<BTreeMap<String, String>> {
        Ok(self
            .honoured()
            .await?
            .into_iter()
            .map(|(param, value)| (param.field.to_string(), value))
            .collect())
    }

    /// Reject anything that must not be written, before any of it is.
    ///
    /// All-or-nothing, matching how the admin API already treats a batch: a partially applied
    /// write would be worse than a clean refusal.
    fn check_writable(updates: &[(String, String)]) -> Result<()> {
        for (field, _) in updates {
            match by_field(field) {
                Some(param) if param.persist == Persist::Allowed => {}
                Some(param) => {
                    return Err(AppError::ConfigNotPersistable(format!(
                        "`{}` cannot be stored in the database",
                        param.field
                    )))
                }
                None => {
                    return Err(AppError::InvalidConfig(format!(
                        "`{field}` is not a configuration field"
                    )))
                }
            }
        }
        Ok(())
    }

    /// Write settings and open a new generation, in one transaction.
    ///
    /// The generation snapshots the table *after* the write, so a later rollback restores what
    /// this write produced rather than what it replaced.
    pub async fn set(&self, updates: &[(String, String)], by: WrittenBy) -> Result<i64> {
        Self::check_writable(updates)?;

        let now = Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;

        for (field, value) in updates {
            sqlx::query(
                r#"
                INSERT INTO config_settings (field, value, updated_at, updated_by)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(field) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at,
                    updated_by = excluded.updated_by
                "#,
            )
            .bind(field)
            .bind(value)
            .bind(now)
            .bind(by.label())
            .execute(&mut *tx)
            .await?;
        }

        let id = Self::open_generation(&mut tx, now).await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Remove one stored setting, opening a generation so the removal can roll back too.
    ///
    /// Returns whether a row existed.
    pub async fn unset(&self, field: &str) -> Result<bool> {
        let now = Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;

        let deleted = sqlx::query("DELETE FROM config_settings WHERE field = ?")
            .bind(field)
            .execute(&mut *tx)
            .await?
            .rows_affected();

        // Only record a generation when something actually changed, so repeatedly deleting a
        // missing field does not fill the table with identical snapshots.
        if deleted > 0 {
            Self::open_generation(&mut tx, now).await?;
        }
        tx.commit().await?;
        Ok(deleted > 0)
    }

    /// Remove every stored setting. The offline escape hatch behind `captchapi config clear`.
    pub async fn clear(&self) -> Result<u64> {
        let now = Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        let deleted = sqlx::query("DELETE FROM config_settings")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if deleted > 0 {
            Self::open_generation(&mut tx, now).await?;
        }
        tx.commit().await?;
        Ok(deleted)
    }

    /// Snapshot `config_settings` inside `tx` and record it as a pending generation.
    async fn open_generation(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        now: i64,
    ) -> Result<i64> {
        let rows = sqlx::query("SELECT field, value FROM config_settings ORDER BY field")
            .fetch_all(&mut **tx)
            .await?;
        let snapshot: BTreeMap<String, String> = rows
            .into_iter()
            .map(|r| (r.get::<String, _>("field"), r.get::<String, _>("value")))
            .collect();
        let json = serde_json::to_string(&snapshot).map_err(|e| {
            AppError::Internal(anyhow::anyhow!("cannot encode config snapshot: {e}"))
        })?;

        let id = sqlx::query(
            "INSERT INTO config_generations (created_at, snapshot, status, attempts)
             VALUES (?, ?, 'pending', 0)",
        )
        .bind(now)
        .bind(json)
        .execute(&mut **tx)
        .await?
        .last_insert_rowid();

        Ok(id)
    }

    /// Decide what this boot should do about the newest generation.
    ///
    /// Called once, after migrations and before the stored layer is read, so a rollback is
    /// already reflected in what [`Self::load`] returns.
    pub async fn prepare_boot(&self) -> Result<BootOutcome> {
        let mut tx = self.pool.begin().await?;

        let newest = sqlx::query(
            "SELECT id, status, attempts FROM config_generations ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = newest else {
            tx.commit().await?;
            return Ok(BootOutcome::Clean { generation: None });
        };

        let id: i64 = row.get("id");
        let status: String = row.get("status");
        let attempts: i64 = row.get("attempts");

        if status != "pending" {
            tx.commit().await?;
            return Ok(BootOutcome::Clean {
                generation: Some(id),
            });
        }

        if attempts == 0 {
            sqlx::query("UPDATE config_generations SET attempts = attempts + 1 WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(BootOutcome::Trying { generation: id });
        }

        // Already tried at least once and we are here again, so the previous attempt did not
        // survive long enough to be confirmed. Restore the last configuration that did.
        let confirmed: Option<String> = sqlx::query(
            "SELECT snapshot FROM config_generations WHERE status = 'confirmed' ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await?
        .map(|r| r.get("snapshot"));

        let restore: BTreeMap<String, String> = match confirmed {
            Some(json) => serde_json::from_str(&json).map_err(|e| {
                AppError::Internal(anyhow::anyhow!("cannot decode config snapshot: {e}"))
            })?,
            // Nothing was ever confirmed, so the safe baseline is no stored configuration at
            // all — which is what the service ran with before any of this was written.
            None => BTreeMap::new(),
        };

        sqlx::query("DELETE FROM config_settings")
            .execute(&mut *tx)
            .await?;
        let now = Utc::now().timestamp();
        for (field, value) in &restore {
            sqlx::query(
                "INSERT INTO config_settings (field, value, updated_at, updated_by)
                 VALUES (?, ?, ?, 'rollback')",
            )
            .bind(field)
            .bind(value)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query("UPDATE config_generations SET status = 'rolled_back' WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(BootOutcome::RolledBack {
            generation: id,
            restored_fields: restore.len(),
        })
    }

    /// Mark a generation as one a process actually managed to run.
    ///
    /// Takes the specific id [`Self::prepare_boot`] reported rather than "whatever is pending",
    /// because a write made *while this process runs* opens a generation this process has not
    /// proved anything about. Confirming that one would disarm the rollback for the very
    /// change most likely to need it.
    pub async fn confirm(&self, generation: i64) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE config_generations SET status = 'confirmed' WHERE id = ? AND status = 'pending'",
        )
        .bind(generation)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        // Every write records a full snapshot, so without this the table grows forever in a
        // database whose whole point is to be small. Confirming a generation makes everything
        // strictly older than it unreachable: rollback only ever consults the newest row and
        // the newest confirmed one, and this row is now both. Pruning here rather than on write
        // is what keeps the last known-good snapshot alive through a burst of failed attempts.
        if updated > 0 {
            let pruned = sqlx::query("DELETE FROM config_generations WHERE id < ?")
                .bind(generation)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if pruned > 0 {
                tracing::debug!("Pruned {pruned} superseded configuration generation(s)");
            }
        }

        tx.commit().await?;
        Ok(updated > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use uuid::Uuid;

    async fn store() -> ConfigStore {
        let db = format!(
            "file:test_cfgstore_{}?mode=memory&cache=shared",
            Uuid::new_v4()
        );
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db)
                    .create_if_missing(true),
            )
            .await
            .expect("test database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        ConfigStore::new(pool)
    }

    fn updates(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn test_a_stored_value_comes_back_as_a_layer_keyed_by_env_name() {
        let store = store().await;
        store
            .set(
                &updates(&[("captcha_compression", "70")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();

        // The table speaks field names, the configuration stack speaks environment keys.
        assert_eq!(store.all().await.unwrap()["captcha_compression"], "70");
        assert_eq!(store.load().await.unwrap()["CAPTCHA_COMPRESSION"], "70");
    }

    #[tokio::test]
    async fn test_set_overwrites_rather_than_duplicating() {
        let store = store().await;
        for value in ["70", "80"] {
            store
                .set(
                    &updates(&[("captcha_compression", value)]),
                    WrittenBy::AdminApi,
                )
                .await
                .unwrap();
        }
        let all = store.all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all["captcha_compression"], "80");
    }

    #[tokio::test]
    async fn test_unstorable_fields_are_refused() {
        let store = store().await;

        let err = store
            .set(&updates(&[("api_key_salt", "leak")]), WrittenBy::AdminApi)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ConfigNotPersistable(_)), "{err:?}");

        let err = store
            .set(
                &updates(&[("database_url", "sqlite:/tmp/x")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ConfigNotPersistable(_)), "{err:?}");

        let err = store
            .set(&updates(&[("not_a_field", "x")]), WrittenBy::AdminApi)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidConfig(_)), "{err:?}");

        assert!(store.all().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_a_batch_is_all_or_nothing() {
        let store = store().await;
        store
            .set(
                &updates(&[("captcha_compression", "70"), ("api_key_salt", "leak")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap_err();
        assert!(
            store.all().await.unwrap().is_empty(),
            "the writable half must not have landed"
        );
    }

    #[tokio::test]
    async fn test_load_drops_a_hand_edited_row_it_must_never_honour() {
        // Nothing this service writes can produce such a row; it can only arrive by editing the
        // database directly. Honouring it would let a write to a data file override a secret.
        let store = store().await;
        for field in ["api_key_salt", "database_url", "not_a_field"] {
            sqlx::query(
                "INSERT INTO config_settings (field, value, updated_at, updated_by)
                 VALUES (?, 'smuggled', 0, 'hand-edit')",
            )
            .bind(field)
            .execute(&store.pool)
            .await
            .unwrap();
        }

        let layer = store.load().await.unwrap();

        assert!(layer.is_empty(), "{layer:?}");
    }

    #[tokio::test]
    async fn test_unset_removes_a_value_and_reports_whether_it_existed() {
        let store = store().await;
        store
            .set(
                &updates(&[("captcha_compression", "70")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();

        assert!(store.unset("captcha_compression").await.unwrap());
        assert!(store.all().await.unwrap().is_empty());
        assert!(!store.unset("captcha_compression").await.unwrap());
    }

    #[tokio::test]
    async fn test_clear_removes_everything() {
        let store = store().await;
        store
            .set(
                &updates(&[("captcha_compression", "70"), ("server_port", "8080")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();

        assert_eq!(store.clear().await.unwrap(), 2);
        assert!(store.all().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_a_fresh_database_has_nothing_pending() {
        let store = store().await;
        assert_eq!(
            store.prepare_boot().await.unwrap(),
            BootOutcome::Clean { generation: None }
        );
    }

    #[tokio::test]
    async fn test_the_first_boot_after_a_write_is_a_trial() {
        let store = store().await;
        let generation = store
            .set(
                &updates(&[("captcha_compression", "70")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();

        assert_eq!(
            store.prepare_boot().await.unwrap(),
            BootOutcome::Trying { generation }
        );
        // ...and the settings are still there to be read.
        assert_eq!(store.load().await.unwrap()["CAPTCHA_COMPRESSION"], "70");
    }

    #[tokio::test]
    async fn test_a_second_boot_without_confirmation_rolls_back() {
        // Models a stored value that stops the process starting: boot, die, boot again.
        let store = store().await;
        store
            .set(
                &updates(&[("captcha_compression", "70")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();
        let first = store.prepare_boot().await.unwrap();
        assert!(matches!(first, BootOutcome::Trying { .. }));

        let second = store.prepare_boot().await.unwrap();

        assert!(
            matches!(
                second,
                BootOutcome::RolledBack {
                    restored_fields: 0,
                    ..
                }
            ),
            "{second:?}"
        );
        assert!(
            store.load().await.unwrap().is_empty(),
            "nothing was ever confirmed, so the baseline is no stored configuration"
        );
    }

    #[tokio::test]
    async fn test_rollback_restores_the_last_confirmed_settings() {
        let store = store().await;

        // A generation that a process ran successfully.
        let good = store
            .set(
                &updates(&[("captcha_compression", "70")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();
        store.prepare_boot().await.unwrap();
        assert!(store.confirm(good).await.unwrap());

        // A later one that never survives.
        store
            .set(&updates(&[("server_port", "1")]), WrittenBy::AdminApi)
            .await
            .unwrap();
        store.prepare_boot().await.unwrap();
        let outcome = store.prepare_boot().await.unwrap();

        assert!(
            matches!(
                outcome,
                BootOutcome::RolledBack {
                    restored_fields: 1,
                    ..
                }
            ),
            "{outcome:?}"
        );
        let layer = store.load().await.unwrap();
        assert_eq!(layer["CAPTCHA_COMPRESSION"], "70");
        assert!(!layer.contains_key("SERVER_PORT"), "{layer:?}");
    }

    #[tokio::test]
    async fn test_a_confirmed_generation_boots_clean_forever() {
        let store = store().await;
        let generation = store
            .set(
                &updates(&[("captcha_compression", "70")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();
        store.prepare_boot().await.unwrap();
        store.confirm(generation).await.unwrap();

        for _ in 0..3 {
            assert_eq!(
                store.prepare_boot().await.unwrap(),
                BootOutcome::Clean {
                    generation: Some(generation)
                }
            );
        }
        assert_eq!(store.load().await.unwrap()["CAPTCHA_COMPRESSION"], "70");
    }

    #[tokio::test]
    async fn test_confirm_only_touches_the_generation_it_is_given() {
        // The reason `confirm` takes an id: a write made while this process runs opens a
        // generation this process has proved nothing about. Confirming it would disarm the
        // rollback for exactly the change most likely to need it.
        let store = store().await;
        let booted = store
            .set(
                &updates(&[("captcha_compression", "70")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();
        store.prepare_boot().await.unwrap();

        let written_later = store
            .set(&updates(&[("server_port", "8080")]), WrittenBy::AdminApi)
            .await
            .unwrap();
        assert_ne!(booted, written_later);

        store.confirm(booted).await.unwrap();

        // The later generation is still pending, so a boot that fails on it can roll back.
        store.prepare_boot().await.unwrap();
        let outcome = store.prepare_boot().await.unwrap();
        assert!(
            matches!(outcome, BootOutcome::RolledBack { .. }),
            "{outcome:?}"
        );
        assert_eq!(store.load().await.unwrap()["CAPTCHA_COMPRESSION"], "70");
    }

    async fn generation_count(store: &ConfigStore) -> i64 {
        sqlx::query("SELECT COUNT(*) AS n FROM config_generations")
            .fetch_one(&store.pool)
            .await
            .unwrap()
            .get("n")
    }

    #[tokio::test]
    async fn test_confirming_prunes_the_generations_it_supersedes() {
        // Every write snapshots the whole table, so without pruning this grows forever in a
        // database whose entire point is to stay small.
        let store = store().await;
        for value in ["10", "20", "30", "40"] {
            store
                .set(
                    &updates(&[("captcha_compression", value)]),
                    WrittenBy::AdminApi,
                )
                .await
                .unwrap();
        }
        assert_eq!(generation_count(&store).await, 4);

        let newest = store
            .set(
                &updates(&[("captcha_compression", "50")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();
        store.prepare_boot().await.unwrap();
        store.confirm(newest).await.unwrap();

        assert_eq!(generation_count(&store).await, 1);
        assert_eq!(store.load().await.unwrap()["CAPTCHA_COMPRESSION"], "50");
    }

    #[tokio::test]
    async fn test_pruning_keeps_the_snapshot_a_rollback_needs() {
        // A burst of failed attempts must not be able to prune away the last known-good
        // configuration, which is the one a rollback restores.
        let store = store().await;
        let good = store
            .set(
                &updates(&[("captcha_compression", "70")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();
        store.prepare_boot().await.unwrap();
        store.confirm(good).await.unwrap();

        for port in ["1", "2", "3"] {
            store
                .set(&updates(&[("server_port", port)]), WrittenBy::AdminApi)
                .await
                .unwrap();
        }

        store.prepare_boot().await.unwrap();
        let outcome = store.prepare_boot().await.unwrap();

        assert!(
            matches!(
                outcome,
                BootOutcome::RolledBack {
                    restored_fields: 1,
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(
            store.load().await.unwrap()["CAPTCHA_COMPRESSION"],
            "70",
            "the confirmed snapshot survived the failed attempts"
        );
    }

    #[tokio::test]
    async fn test_confirming_twice_is_harmless() {
        let store = store().await;
        let generation = store
            .set(
                &updates(&[("captcha_compression", "70")]),
                WrittenBy::AdminApi,
            )
            .await
            .unwrap();
        store.prepare_boot().await.unwrap();

        assert!(store.confirm(generation).await.unwrap());
        assert!(
            !store.confirm(generation).await.unwrap(),
            "already confirmed, so nothing was updated"
        );
    }
}
