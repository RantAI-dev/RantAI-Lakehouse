//! The deployment's reporting settings (migrations `0070` and `0072`): the
//! report time zone, the first day of the week, and (`BI-17` part A) the
//! number locale, the company palette and the allowed fonts.
//!
//! At most one row exists (`singleton`). No row is not an error: it means the
//! deployment never saved the settings, and the caller supplies the defaults
//! (`lakehouse_bi::grain::TimeContext::default`). This module stores two
//! strings and knows nothing about what a valid zone is; the route checks the
//! zone against the engine's list and the week start against the two values
//! the SQL builder implements, and the table's `CHECK`s are the backstop for
//! shape. The same holds for the formatting columns: the route validates the
//! locale, the palette and the fonts, and this module stores them.

use crate::{PgPool, StoreError};

/// The saved settings, as stored.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ReportingSettings {
    /// An IANA zone name.
    pub time_zone: String,
    /// `"monday"` or `"sunday"`.
    pub week_start: String,
    /// `"en-US"` or `"id-ID"` (`BI-17`, decision 2).
    pub number_locale: String,
    /// The company palette: empty, or 2 to 12 `#rrggbb` colours.
    pub company_palette: Vec<String>,
    /// The font ids dashboards may use, a subset of the console's shipped ones.
    pub fonts: Vec<String>,
}

/// The saved settings, or `None` when the deployment never saved them.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool) -> Result<Option<ReportingSettings>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT time_zone, week_start, number_locale, company_palette, fonts \
             FROM reporting_settings",
    )
    .fetch_optional(pool)
    .await?)
}

/// Save the settings, replacing any earlier ones, and return what is stored.
/// `updated_by` is the saving principal's id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails, including when the
/// table's `CHECK`s refuse the values.
pub async fn upsert(
    pool: &PgPool,
    settings: &ReportingSettings,
    updated_by: &str,
) -> Result<ReportingSettings, StoreError> {
    Ok(sqlx::query_as(
        "INSERT INTO reporting_settings \
             (singleton, time_zone, week_start, number_locale, company_palette, fonts, \
              updated_at, updated_by) \
         VALUES (TRUE, $1, $2, $3, $4, $5, now(), $6) \
         ON CONFLICT (singleton) DO UPDATE SET time_zone = EXCLUDED.time_zone, \
         week_start = EXCLUDED.week_start, number_locale = EXCLUDED.number_locale, \
         company_palette = EXCLUDED.company_palette, fonts = EXCLUDED.fonts, \
         updated_at = now(), updated_by = EXCLUDED.updated_by \
         RETURNING time_zone, week_start, number_locale, company_palette, fonts",
    )
    .bind(&settings.time_zone)
    .bind(&settings.week_start)
    .bind(&settings.number_locale)
    .bind(&settings.company_palette)
    .bind(&settings.fonts)
    .bind(updated_by)
    .fetch_one(pool)
    .await?)
}
