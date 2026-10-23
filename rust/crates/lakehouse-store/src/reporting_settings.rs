//! The deployment's reporting settings (migrations `0070`, `0086`, `0087`, `0088` and `0089`): the
//! report time zone, the first day of the week, and (`BI-17` part A) the
//! number locale, the company palette and the allowed fonts, and (`BI-16`
//! part B) the sites image and web cards may load from, and (`BI-1`) the
//! roles that may not download, and (`RPT-1`) the outside
//! email domains a report may go to.
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
    /// The sites image and web cards may load from: `https://host[:port]`,
    /// at most 50 (`BI-16` part B, migration `0087`).
    pub content_sites: Vec<String>,
    /// Role names whose holders may not download (`BI-1`, migration `0088`).
    /// Empty: every role may.
    pub download_denied_roles: Vec<String>,
    /// The outside email domains a scheduled report may go to (`RPT-1`,
    /// migration `0089`): lower-case names, at most 50. Empty: no report
    /// leaves the company.
    pub report_allowed_domains: Vec<String>,
}

/// The saved settings, or `None` when the deployment never saved them.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool) -> Result<Option<ReportingSettings>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT time_zone, week_start, number_locale, company_palette, fonts, content_sites, \
             download_denied_roles, report_allowed_domains FROM reporting_settings",
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
             (singleton, time_zone, week_start, number_locale, company_palette, fonts, content_sites, \
              download_denied_roles, report_allowed_domains, updated_at, updated_by) \
         VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, $8, now(), $9) \
         ON CONFLICT (singleton) DO UPDATE SET time_zone = EXCLUDED.time_zone, \
         week_start = EXCLUDED.week_start, number_locale = EXCLUDED.number_locale, \
         company_palette = EXCLUDED.company_palette, fonts = EXCLUDED.fonts, \
         content_sites = EXCLUDED.content_sites, \
         download_denied_roles = EXCLUDED.download_denied_roles, \
         report_allowed_domains = EXCLUDED.report_allowed_domains, updated_at = now(), updated_by = EXCLUDED.updated_by \
         RETURNING time_zone, week_start, number_locale, company_palette, fonts, \
         content_sites, download_denied_roles, report_allowed_domains",
    )
    .bind(&settings.time_zone)
    .bind(&settings.week_start)
    .bind(&settings.number_locale)
    .bind(&settings.company_palette)
    .bind(&settings.fonts)
    .bind(&settings.content_sites)
    .bind(&settings.download_denied_roles)
    .bind(&settings.report_allowed_domains)
    .bind(updated_by)
    .fetch_one(pool)
    .await?)
}
