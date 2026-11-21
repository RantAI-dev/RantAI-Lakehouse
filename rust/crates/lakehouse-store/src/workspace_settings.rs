//! The workspace's own name, logo, dark-mode logo and browser-tab icon, saved
//! from Settings → General (`ADM-2`, migration `0147`).
//!
//! At most one row exists (`singleton`). No row is not an error: it means
//! nothing was ever saved, and the console shows the product's own name and
//! pictures. Every field is `Option` for the same reason: `None` is "not set".
//!
//! # The table is the last line
//!
//! The name length, each file's size and each picture's type are `CHECK`s of
//! the table, so a value outside the rules never reaches a row even if a
//! caller skips the API's validation. This module neither inspects a file nor
//! decides its type: the route reads the first bytes (`lakehouse-api`
//! `routes/settings.rs`) and passes the type it found.
//!
//! # `version`
//!
//! Each picture carries `version`, the first 16 hex digits of the SHA-256 of
//! its bytes, computed by Postgres so the bytes are hashed once per read of
//! the row and never in a caller's request path. The console puts it in the
//! picture's address (`?v=`) to bypass a cached older picture, and the API
//! uses it as the `ETag`.
//!
//! # The name a save replaced
//!
//! [`upsert_name`] reads the name its write replaces under the row's lock,
//! in the same transaction as the write, and returns it beside the row it
//! wrote ([`SavedWorkspaceName`]). A caller that wants to know whether the
//! name changed cannot read the row first and compare afterwards: two saves
//! that race would compare against each other's value, and the answer would
//! be about a name that was never replaced (`ADM-2`, plan adm-120 step 3b).

use std::sync::Arc;

use time::OffsetDateTime;

use crate::{PgPool, StoreError};

/// Which of the three pictures a call is about.
///
/// A closed set, matched to constant SQL below: a caller's text never becomes
/// part of a statement or a column name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// The logo shown on light backgrounds.
    Logo,
    /// The logo shown on dark backgrounds.
    LogoDark,
    /// The browser-tab icon.
    Icon,
}

/// One stored picture. The bytes are shared (`Arc`), so cloning the row out
/// of the API's settings cache does not copy up to half a megabyte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredImage {
    /// The media type to serve it with.
    pub content_type: String,
    /// The file.
    pub bytes: Arc<[u8]>,
    /// The first 16 hex digits of the SHA-256 of `bytes`.
    pub version: String,
}

/// The saved workspace settings, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSettings {
    /// The workspace's name, 1 to 60 characters, or `None` for the product's own.
    pub workspace_name: Option<String>,
    /// The logo, or `None`.
    pub logo: Option<StoredImage>,
    /// The dark-mode logo, or `None`.
    pub logo_dark: Option<StoredImage>,
    /// The browser-tab icon, or `None`.
    pub icon: Option<StoredImage>,
    /// When the row was last saved.
    pub updated_at: OffsetDateTime,
}

/// What a name save wrote, and the name it replaced.
///
/// `previous_name` is the name the row held when the write landed. The store
/// reads it under the row's lock, in the same transaction as the write, so a
/// caller that says whether the name changed never has to compare against a
/// value it read separately: such a read can be the value another save wrote
/// in between, and the answer would then be about a name that was never
/// replaced (`ADM-2`, plan adm-120 step 3b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedWorkspaceName {
    /// The row as stored after the write.
    pub settings: WorkspaceSettings,
    /// The name the write replaced, or `None` when there was none.
    pub previous_name: Option<String>,
}

/// The columns every read and every `RETURNING` selects, written once so a
/// new column cannot be added to one and forgotten in another.
macro_rules! columns {
    () => {
        "workspace_name, \
         logo_bytes, logo_type, left(encode(sha256(logo_bytes), 'hex'), 16) AS logo_version, \
         logo_dark_bytes, logo_dark_type, \
         left(encode(sha256(logo_dark_bytes), 'hex'), 16) AS logo_dark_version, \
         icon_bytes, icon_type, left(encode(sha256(icon_bytes), 'hex'), 16) AS icon_version, \
         updated_at"
    };
}

/// One picture's three columns as the query returns them: both of bytes and
/// type are set or both are NULL (a `CHECK`), and the version follows the bytes.
#[derive(sqlx::FromRow)]
struct Row {
    workspace_name: Option<String>,
    logo_bytes: Option<Vec<u8>>,
    logo_type: Option<String>,
    logo_version: Option<String>,
    logo_dark_bytes: Option<Vec<u8>>,
    logo_dark_type: Option<String>,
    logo_dark_version: Option<String>,
    icon_bytes: Option<Vec<u8>>,
    icon_type: Option<String>,
    icon_version: Option<String>,
    updated_at: OffsetDateTime,
}

fn image(
    bytes: Option<Vec<u8>>,
    content_type: Option<String>,
    version: Option<String>,
) -> Option<StoredImage> {
    match (bytes, content_type, version) {
        (Some(bytes), Some(content_type), Some(version)) => Some(StoredImage {
            content_type,
            bytes: bytes.into(),
            version,
        }),
        _ => None,
    }
}

impl From<Row> for WorkspaceSettings {
    fn from(row: Row) -> Self {
        Self {
            workspace_name: row.workspace_name,
            logo: image(row.logo_bytes, row.logo_type, row.logo_version),
            logo_dark: image(
                row.logo_dark_bytes,
                row.logo_dark_type,
                row.logo_dark_version,
            ),
            icon: image(row.icon_bytes, row.icon_type, row.icon_version),
            updated_at: row.updated_at,
        }
    }
}

/// The saved settings, or `None` when nothing was ever saved.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool) -> Result<Option<WorkspaceSettings>, StoreError> {
    let row: Option<Row> =
        sqlx::query_as(concat!("SELECT ", columns!(), " FROM workspace_settings"))
            .fetch_optional(pool)
            .await?;
    Ok(row.map(Into::into))
}

/// Save the name (or clear it with `None`), leaving the pictures as they are,
/// and return what is stored together with the name the write replaced.
/// `updated_by` is the saving principal's id.
///
/// The name the write replaced is read in the SAME transaction as the write,
/// under the row's lock, so it is the value the write really replaced even
/// when another save lands at the same time. A caller that reads the row
/// first and compares afterwards cannot say that: its read is the value
/// before the other save committed, not the value its own write replaced.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails, including when the
/// table's `CHECK` refuses a name that is empty or longer than 60 characters.
pub async fn upsert_name(
    pool: &PgPool,
    name: Option<&str>,
    updated_by: &str,
) -> Result<SavedWorkspaceName, StoreError> {
    let mut tx = pool.begin().await?;
    // The row may not exist yet (nothing was ever saved), so it is created
    // first. `DO NOTHING` never overwrites a row a concurrent save created,
    // and this statement waits for that creator to commit, so the read
    // below finds the row the other save wrote. Every save takes the same
    // three steps in the same order, so two saves of a row that does not
    // exist yet cannot deadlock: the second one's insert waits for the
    // first one to commit, and it takes the row's lock only after that.
    sqlx::query(
        "INSERT INTO workspace_settings (singleton) VALUES (TRUE) \
         ON CONFLICT (singleton) DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    // The row's lock, taken before the write. Under READ COMMITTED this
    // read re-reads the row after it waits for a concurrent writer, so the
    // value it returns IS the value the write below replaces. A plain
    // `SELECT` would not: it reads the snapshot taken before the wait and
    // would report the value that was there before the other save
    // committed.
    let previous_name: Option<String> = sqlx::query_scalar(
        "SELECT workspace_name FROM workspace_settings WHERE singleton = TRUE FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await?;
    // The write itself, in the same transaction and under the lock the read
    // above took, so no other save can write between the two.
    let row: Row = sqlx::query_as(concat!(
        "UPDATE workspace_settings SET workspace_name = $1, updated_at = now(), updated_by = $2 \
         WHERE singleton = TRUE RETURNING ",
        columns!()
    ))
    .bind(name)
    .bind(updated_by)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(SavedWorkspaceName {
        settings: row.into(),
        previous_name,
    })
}

/// The statement that stores one picture, written out per kind so each column
/// name is a literal in the source (never built from text at run time).
macro_rules! set_sql {
    ($bytes:literal, $type:literal) => {
        concat!(
            "INSERT INTO workspace_settings (singleton, ",
            $bytes,
            ", ",
            $type,
            ", updated_at, updated_by) VALUES (TRUE, $1, $2, now(), $3) \
             ON CONFLICT (singleton) DO UPDATE SET ",
            $bytes,
            " = EXCLUDED.",
            $bytes,
            ", ",
            $type,
            " = EXCLUDED.",
            $type,
            ", updated_at = now(), updated_by = EXCLUDED.updated_by RETURNING ",
            columns!()
        )
    };
}

/// The statement that removes one picture.
macro_rules! clear_sql {
    ($bytes:literal, $type:literal) => {
        concat!(
            "INSERT INTO workspace_settings (singleton, updated_at, updated_by) \
             VALUES (TRUE, now(), $1) \
             ON CONFLICT (singleton) DO UPDATE SET ",
            $bytes,
            " = NULL, ",
            $type,
            " = NULL, updated_at = now(), updated_by = EXCLUDED.updated_by RETURNING ",
            columns!()
        )
    };
}

/// Store one picture, replacing the earlier one of that kind, and return what
/// is stored. `content_type` is the type the caller found in the file's first
/// bytes. `updated_by` is the saving principal's id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails, including when the
/// table's `CHECK`s refuse the size or the type for this kind.
pub async fn set_image(
    pool: &PgPool,
    kind: ImageKind,
    content_type: &str,
    bytes: &[u8],
    updated_by: &str,
) -> Result<WorkspaceSettings, StoreError> {
    let sql = match kind {
        ImageKind::Logo => set_sql!("logo_bytes", "logo_type"),
        ImageKind::LogoDark => set_sql!("logo_dark_bytes", "logo_dark_type"),
        ImageKind::Icon => set_sql!("icon_bytes", "icon_type"),
    };
    let row: Row = sqlx::query_as(sql)
        .bind(bytes)
        .bind(content_type)
        .bind(updated_by)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Remove one picture (nothing to remove is not an error) and return what is
/// stored. `updated_by` is the saving principal's id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails.
pub async fn clear_image(
    pool: &PgPool,
    kind: ImageKind,
    updated_by: &str,
) -> Result<WorkspaceSettings, StoreError> {
    let sql = match kind {
        ImageKind::Logo => clear_sql!("logo_bytes", "logo_type"),
        ImageKind::LogoDark => clear_sql!("logo_dark_bytes", "logo_dark_type"),
        ImageKind::Icon => clear_sql!("icon_bytes", "icon_type"),
    };
    let row: Row = sqlx::query_as(sql).bind(updated_by).fetch_one(pool).await?;
    Ok(row.into())
}

/// Clear the name and all three pictures. The row is deleted, so the state
/// afterwards is the one of a workspace that never saved anything.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails.
pub async fn reset(pool: &PgPool) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM workspace_settings")
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const PNG: &str = "image/png";

    /// The sizes the table allows, from the migration's `CHECK`s.
    const LOGO_MAX: usize = 524_288;
    const ICON_MAX: usize = 65_536;

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_saved_name_and_pictures_round_trip_with_a_version(pool: PgPool) -> sqlx::Result<()> {
        assert!(
            get(&pool).await.unwrap().is_none(),
            "no row before the first save"
        );

        upsert_name(&pool, Some("Meridian Data"), "user-1")
            .await
            .unwrap();
        set_image(&pool, ImageKind::Logo, PNG, b"logo-bytes", "user-1")
            .await
            .unwrap();
        let saved = set_image(&pool, ImageKind::Icon, PNG, b"icon-bytes", "user-1")
            .await
            .unwrap();

        let fetched = get(&pool).await.unwrap().unwrap();
        assert_eq!(fetched, saved);
        assert_eq!(fetched.workspace_name.as_deref(), Some("Meridian Data"));
        let logo = fetched.logo.expect("the logo was saved");
        assert_eq!(logo.content_type, PNG);
        assert_eq!(&*logo.bytes, b"logo-bytes");
        assert_eq!(logo.version.len(), 16, "{}", logo.version);
        assert!(logo.version.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(fetched.logo_dark.is_none(), "the dark logo was not saved");
        assert_eq!(&*fetched.icon.unwrap().bytes, b"icon-bytes");
        Ok(())
    }

    /// The version is what the console puts in the picture's address to
    /// bypass a cached older picture, so it moves with the bytes and only
    /// with them.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_version_follows_the_bytes(pool: PgPool) -> sqlx::Result<()> {
        let first = set_image(&pool, ImageKind::Logo, PNG, b"one", "u")
            .await
            .unwrap();
        let same = set_image(&pool, ImageKind::Logo, PNG, b"one", "u")
            .await
            .unwrap();
        let other = set_image(&pool, ImageKind::Logo, PNG, b"two", "u")
            .await
            .unwrap();
        assert_eq!(
            first.logo.as_ref().unwrap().version,
            same.logo.unwrap().version
        );
        assert_ne!(first.logo.unwrap().version, other.logo.unwrap().version);
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_table_refuses_a_name_outside_one_to_sixty_characters(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        assert!(
            upsert_name(&pool, Some(&"a".repeat(61)), "u")
                .await
                .is_err(),
            "61 characters"
        );
        assert!(upsert_name(&pool, Some(""), "u").await.is_err(), "empty");
        let sixty = "é".repeat(60);
        let saved = upsert_name(&pool, Some(&sixty), "u").await.unwrap();
        assert_eq!(
            saved.settings.workspace_name.as_deref(),
            Some(sixty.as_str()),
            "60 characters, counted as characters and not bytes"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_table_refuses_a_picture_over_its_size(pool: PgPool) -> sqlx::Result<()> {
        for kind in [ImageKind::Logo, ImageKind::LogoDark] {
            assert!(
                set_image(&pool, kind, PNG, &vec![0; LOGO_MAX + 1], "u")
                    .await
                    .is_err(),
                "{kind:?} one byte over 512 KB"
            );
            set_image(&pool, kind, PNG, &vec![0; LOGO_MAX], "u")
                .await
                .unwrap();
        }
        assert!(
            set_image(&pool, ImageKind::Icon, PNG, &vec![0; ICON_MAX + 1], "u")
                .await
                .is_err(),
            "the icon, one byte over 64 KB"
        );
        set_image(&pool, ImageKind::Icon, PNG, &vec![0; ICON_MAX], "u")
            .await
            .unwrap();
        assert!(
            set_image(&pool, ImageKind::Logo, PNG, &[], "u")
                .await
                .is_err(),
            "an empty file is no picture"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_table_refuses_a_type_outside_its_list(pool: PgPool) -> sqlx::Result<()> {
        assert!(
            set_image(&pool, ImageKind::Logo, "image/svg+xml", b"x", "u")
                .await
                .is_err()
        );
        assert!(
            set_image(&pool, ImageKind::Icon, "image/jpeg", b"x", "u")
                .await
                .is_err(),
            "the icon is PNG only"
        );
        for accepted in ["image/png", "image/jpeg", "image/webp"] {
            set_image(&pool, ImageKind::LogoDark, accepted, b"x", "u")
                .await
                .unwrap();
        }
        Ok(())
    }

    /// Bytes without a type, or a type without bytes, never reach a row.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_table_refuses_a_half_set_picture(pool: PgPool) -> sqlx::Result<()> {
        upsert_name(&pool, None, "u").await.unwrap();
        let type_only = sqlx::query("UPDATE workspace_settings SET logo_type = 'image/png'")
            .execute(&pool)
            .await;
        assert!(type_only.is_err(), "a type with no bytes");
        let bytes_only = sqlx::query("UPDATE workspace_settings SET icon_bytes = '\\x00'::bytea")
            .execute(&pool)
            .await;
        assert!(bytes_only.is_err(), "bytes with no type");
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn clearing_one_picture_leaves_the_rest(pool: PgPool) -> sqlx::Result<()> {
        upsert_name(&pool, Some("Name"), "u").await.unwrap();
        set_image(&pool, ImageKind::Logo, PNG, b"a", "u")
            .await
            .unwrap();
        set_image(&pool, ImageKind::LogoDark, PNG, b"b", "u")
            .await
            .unwrap();
        let after = clear_image(&pool, ImageKind::Logo, "u").await.unwrap();
        assert!(after.logo.is_none());
        assert_eq!(&*after.logo_dark.unwrap().bytes, b"b");
        assert_eq!(after.workspace_name.as_deref(), Some("Name"));
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn clearing_the_name_leaves_the_pictures(pool: PgPool) -> sqlx::Result<()> {
        upsert_name(&pool, Some("Name"), "u").await.unwrap();
        set_image(&pool, ImageKind::Logo, PNG, b"a", "u")
            .await
            .unwrap();
        let after = upsert_name(&pool, None, "u").await.unwrap();
        assert!(after.settings.workspace_name.is_none());
        assert_eq!(&*after.settings.logo.unwrap().bytes, b"a");
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reset_clears_the_name_and_every_picture(pool: PgPool) -> sqlx::Result<()> {
        upsert_name(&pool, Some("Name"), "u").await.unwrap();
        for kind in [ImageKind::Logo, ImageKind::LogoDark, ImageKind::Icon] {
            set_image(&pool, kind, PNG, b"x", "u").await.unwrap();
        }
        reset(&pool).await.unwrap();
        assert!(get(&pool).await.unwrap().is_none());
        reset(&pool).await.unwrap();
        Ok(())
    }

    /// The name a save replaced is the name the row held, not the name the
    /// save wrote: a caller that says whether the name changed needs the
    /// value its own write replaced, and only the store can say it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_name_save_reports_the_name_it_replaced(pool: PgPool) -> sqlx::Result<()> {
        let first = upsert_name(&pool, Some("Meridian"), "u").await.unwrap();
        assert_eq!(
            first.previous_name, None,
            "nothing was saved before, so there was no name to replace"
        );
        assert_eq!(first.settings.workspace_name.as_deref(), Some("Meridian"));

        let same = upsert_name(&pool, Some("Meridian"), "u").await.unwrap();
        assert_eq!(
            same.previous_name.as_deref(),
            Some("Meridian"),
            "the second save replaced the name the first one wrote"
        );

        let other = upsert_name(&pool, Some("Atlas"), "u").await.unwrap();
        assert_eq!(other.previous_name.as_deref(), Some("Meridian"));
        assert_eq!(other.settings.workspace_name.as_deref(), Some("Atlas"));

        let cleared = upsert_name(&pool, None, "u").await.unwrap();
        assert_eq!(cleared.previous_name.as_deref(), Some("Atlas"));
        assert!(cleared.settings.workspace_name.is_none());
        Ok(())
    }
}
