//! `GET`/`PUT /api/settings/reporting` — the deployment's report time zone
//! and first day of the week (`BI-9`), and the number locale, company palette
//! and allowed fonts (`BI-17` part A, migration `0072`), and the sites image
//! and web cards may load from (`BI-16` part B, migration `0073`).
//!
//! # Why one deployment-wide setting
//!
//! Grouping a timestamp by day or month, and the relative date filters
//! ("this month", "last 7 days"), both need to know which zone's midnight
//! counts. One zone for everybody keeps a chart and a filter from disagreeing
//! (feature page `docs/core/features/time-grain.md`, "Limits to tell a
//! customer"). Stored in Postgres (`reporting_settings`, migration `0070`);
//! with no row saved the defaults apply (`Asia/Jakarta`, Monday).
//!
//! # Who may do what
//!
//! Reading needs only a sign-in: the console reads the zone and the first day
//! to label axes. Changing needs `settings:write`, a new permission string
//! that follows the rules at the top of `policy.rs`: no seed grant, only a
//! `*:*` holder (Platform Admin) has it until an operator grants it to a role.
//!
//! # The formatting fields
//!
//! `numberLocale` (`en-US` or `id-ID`), `companyPalette` (empty, or 2 to 12
//! `#rrggbb`) and `fonts` (a subset of the console's shipped fonts) are
//! optional on the body: one that is not sent keeps the value stored (or the
//! default), so the time zone form of `BI-9` still works unchanged. They
//! reach every view through the dashboard payload's `reporting` object
//! ([`add_formatting`]), so a public link and a signed embed format numbers
//! and pick colours the way the console does.
//!
//! # Content sites
//!
//! `contentSites` (0 to 50 entries, `https://host[:port]`, no wildcard) is the
//! list an image or web card's address must belong to
//! ([`lakehouse_bi::embed_access::validate_content_sites`], the same parser as
//! the signed-embed sites). It is optional like the formatting fields, rides in
//! the `reporting` object so every view applies the same list, and an empty
//! list (the default) means no such card can be saved: fail closed. The
//! console and the assistant check a card's address against it at save
//! (`lakehouse_bi::types_b::check_content_url`).
//!
//! # The zone is checked against the engine
//!
//! A name is saved only if it has the shape of a zone name
//! ([`lakehouse_bi::grain::TimeZoneName`]) and `system.time_zones` lists it,
//! so a typo cannot reach every dashboard's SQL. That table is readable by a
//! user with `SELECT` on one database only (measured on `ClickHouse` 26.8), so
//! the check needs no privilege the API's user lacks. A failure of the engine
//! or the database goes through `upstream_error`.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use lakehouse_auth::Principal;
use lakehouse_bi::embed_access::validate_content_sites;
use lakehouse_bi::formatting::{COMPANY_PALETTE_LEN, NUMBER_LOCALES, SHIPPED_FONTS, is_hex_color};
use lakehouse_bi::grain::{TimeContext, WeekStart};
use lakehouse_core::ApiError;
use lakehouse_core::ident::SqlLiteral;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upstream_error;

/// What the person is told when `contentSites` is not a list of sites.
const CONTENT_SITES_REFUSED: &str = "contentSites must be at most 50 sites, each written \
https://host or https://host:port, with no wildcard, path or user name.";

/// What the person is told when the body is not the two fields.
const BODY_SHAPE: &str = "body must be {\"timeZone\": string, \"weekStart\": \"monday\"|\"sunday\", \
\"numberLocale\"?: \"en-US\"|\"id-ID\", \"companyPalette\"?: [\"#rrggbb\", ...], \"fonts\"?: [string, ...], \"contentSites\"?: [\"https://host\", ...]}";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsBody {
    time_zone: String,
    week_start: String,
    number_locale: Option<String>,
    company_palette: Option<Vec<String>>,
    fonts: Option<Vec<String>>,
    content_sites: Option<Vec<String>>,
}

/// The formatting half of the settings, as the console reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormatSettings {
    pub number_locale: String,
    pub company_palette: Vec<String>,
    pub fonts: Vec<String>,
    pub content_sites: Vec<String>,
}

impl Default for FormatSettings {
    /// What a deployment that never saved gets: `en-US` separators, no
    /// company palette, every shipped font allowed (so no chart changes).
    fn default() -> Self {
        Self {
            number_locale: NUMBER_LOCALES[0].to_owned(),
            company_palette: Vec::new(),
            fonts: SHIPPED_FONTS.iter().map(|f| (*f).to_owned()).collect(),
            content_sites: Vec::new(),
        }
    }
}

impl FormatSettings {
    fn from_row(row: &lakehouse_store::reporting_settings::ReportingSettings) -> Self {
        Self {
            number_locale: row.number_locale.clone(),
            company_palette: row.company_palette.clone(),
            fonts: row.fonts.clone(),
            content_sites: row.content_sites.clone(),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "numberLocale": self.number_locale,
            "companyPalette": self.company_palette,
            "fonts": self.fonts,
            "contentSites": self.content_sites,
        })
    }
}

/// The formatting settings every dashboard read carries, loaded once per
/// request. No Postgres pool or no saved row means the defaults.
///
/// # Errors
///
/// A database failure is reported through `upstream_error`.
pub(crate) async fn format_settings(state: &AppState) -> Result<FormatSettings, ApiError> {
    let Some(pool) = state.pg.as_deref() else {
        return Ok(FormatSettings::default());
    };
    let saved = lakehouse_store::reporting_settings::get(pool)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    Ok(saved
        .as_ref()
        .map_or_else(FormatSettings::default, FormatSettings::from_row))
}

/// Add the formatting settings to a dashboard payload's `reporting` object
/// (`BI-17` part A, D5), so the console, a public link and a signed embed all
/// read them from the one place `BI-9` put the time zone.
pub(crate) fn add_formatting(body: &mut Value, format: &FormatSettings) {
    let Some(reporting) = body.get_mut("reporting").and_then(Value::as_object_mut) else {
        return;
    };
    if let Value::Object(fields) = format.to_json() {
        reporting.extend(fields);
    }
}

/// Check the optional formatting fields of a body against the closed lists,
/// with the sentence the person is told.
fn checked_formatting(
    parsed: &SettingsBody,
    current: &FormatSettings,
) -> Result<FormatSettings, ApiError> {
    let number_locale = match &parsed.number_locale {
        Some(l) if NUMBER_LOCALES.contains(&l.as_str()) => l.clone(),
        Some(_) => {
            return Err(ApiError::BadRequest(
                "numberLocale must be en-US or id-ID.".to_owned(),
            ));
        }
        None => current.number_locale.clone(),
    };
    let company_palette = match &parsed.company_palette {
        Some(p) if p.is_empty() => Vec::new(),
        Some(p) if !COMPANY_PALETTE_LEN.contains(&p.len()) => {
            return Err(ApiError::BadRequest(
                "The company palette needs 2 to 12 colours, or none.".to_owned(),
            ));
        }
        Some(p) if !p.iter().all(|c| is_hex_color(c)) => {
            return Err(ApiError::BadRequest(
                "Every company palette colour must look like #1a2b3c.".to_owned(),
            ));
        }
        Some(p) => p.iter().map(|c| c.to_ascii_lowercase()).collect(),
        None => current.company_palette.clone(),
    };
    let fonts = match &parsed.fonts {
        Some(f) if f.iter().all(|id| SHIPPED_FONTS.contains(&id.as_str())) => {
            // Shipped order, no duplicates: the stored list is a set.
            SHIPPED_FONTS
                .iter()
                .filter(|id| f.iter().any(|x| x == **id))
                .map(|id| (*id).to_owned())
                .collect()
        }
        Some(_) => {
            return Err(ApiError::BadRequest(
                "fonts must be chosen from the fonts the console ships.".to_owned(),
            ));
        }
        None => current.fonts.clone(),
    };
    let content_sites = match &parsed.content_sites {
        // A fixed sentence: the validator names the first bad entry, and an
        // entry is the administrator's own text, but a response never echoes
        // text built by a validator (SEC-11's guard reads this line).
        Some(sites) => validate_content_sites(sites)
            .map_err(|_| ApiError::BadRequest(CONTENT_SITES_REFUSED.to_owned()))?,
        None => current.content_sites.clone(),
    };
    Ok(FormatSettings {
        number_locale,
        company_palette,
        fonts,
        content_sites,
    })
}

/// The time context every dashboard read uses, loaded once per request.
///
/// No Postgres pool or no saved row means the defaults, not an error: a
/// deployment that never opened Settings behaves as it was told it would.
///
/// # Errors
///
/// A database failure is reported through `upstream_error` (no fabricated
/// default, which would shift every date silently); a stored row that no
/// longer validates (edited by hand around the CHECKs) is a fixed `500`.
pub(crate) async fn time_context(state: &AppState) -> Result<TimeContext, ApiError> {
    let Some(pool) = state.pg.as_deref() else {
        return Ok(TimeContext::default());
    };
    let saved = lakehouse_store::reporting_settings::get(pool)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    let Some(saved) = saved else {
        return Ok(TimeContext::default());
    };
    let week_start = WeekStart::parse(&saved.week_start);
    week_start
        .and_then(|w| TimeContext::new(&saved.time_zone, w).ok())
        // BI-16 part B: the content sites ride in the context every chart
        // save already receives (see `TimeContext`).
        .map(|ctx| ctx.with_content_sites(saved.content_sites.clone()))
        .ok_or_else(|| ApiError::Internal("The reporting settings are invalid.".to_owned()))
}

fn body_json(ctx: &TimeContext, format: &FormatSettings, saved: bool) -> Value {
    let mut body = json!({
        "timeZone": ctx.zone(),
        "weekStart": ctx.week_start().as_str(),
        "saved": saved,
    });
    if let (Some(map), Value::Object(fields)) = (body.as_object_mut(), format.to_json()) {
        map.extend(fields);
    }
    body
}

/// `GET /api/settings/reporting` — the zone and first day in use, and whether
/// they were ever saved (`saved: false` means these are the defaults).
///
/// # Errors
///
/// 5xx from [`time_context`] when the settings cannot be read.
pub async fn get_reporting(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let ctx = time_context(&state).await?;
    let format = format_settings(&state).await?;
    let saved = match state.pg.as_deref() {
        Some(pool) => lakehouse_store::reporting_settings::get(pool)
            .await
            .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?
            .is_some(),
        None => false,
    };
    Ok(ApiJson(body_json(&ctx, &format, saved)))
}

/// `PUT /api/settings/reporting` — save `{timeZone, weekStart}` and, when
/// sent, `{numberLocale, companyPalette, fonts, contentSites}`.
///
/// # Errors
///
/// 400 with a fixed message when the body is not those fields, the first
/// day is not `monday` or `sunday`, the locale, palette, fonts or content
/// sites are outside their lists, or the engine does not know the zone;
/// 503 without a Postgres pool; 5xx through `upstream_error` when the engine
/// or the database fails.
pub async fn put_reporting(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    // Parsed by hand so a malformed body gets this module's message.
    let parsed: SettingsBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(BODY_SHAPE.to_owned()))?;
    let week_start = WeekStart::parse(&parsed.week_start)
        .ok_or_else(|| ApiError::BadRequest("weekStart must be monday or sunday.".to_owned()))?;
    let ctx = TimeContext::new(&parsed.time_zone, week_start)
        .map_err(|_| ApiError::BadRequest("Unknown time zone.".to_owned()))?;
    let Some(pool) = state.pg.as_deref() else {
        return Err(ApiError::Unavailable(
            "Settings need a database, and none is configured.".to_owned(),
        )
        .into());
    };
    let current = format_settings(&state).await?;
    let format = checked_formatting(&parsed, &current)?;
    if !engine_knows_zone(&state, ctx.zone()).await? {
        return Err(ApiError::BadRequest("Unknown time zone.".to_owned()).into());
    }
    let stored = lakehouse_store::reporting_settings::upsert(
        pool,
        &lakehouse_store::reporting_settings::ReportingSettings {
            time_zone: ctx.zone().to_owned(),
            week_start: week_start.as_str().to_owned(),
            number_locale: format.number_locale,
            company_palette: format.company_palette,
            fonts: format.fonts,
            content_sites: format.content_sites,
        },
        &principal.id.uuid().to_string(),
    )
    .await
    .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    // Echo what was stored, through the same constructor reads use.
    let echoed = WeekStart::parse(&stored.week_start)
        .and_then(|w| TimeContext::new(&stored.time_zone, w).ok())
        .unwrap_or(ctx);
    Ok(ApiJson(body_json(
        &echoed,
        &FormatSettings::from_row(&stored),
        true,
    )))
}

/// Whether `system.time_zones` lists `zone`. `zone` already has the shape of
/// a zone name and is printed as a literal.
async fn engine_knows_zone(state: &AppState, zone: &str) -> Result<bool, ApiError> {
    let sql = format!(
        "SELECT count() AS n FROM system.time_zones WHERE time_zone = {}",
        SqlLiteral::from(zone)
    );
    let rows = state
        .clickhouse
        .rows(&sql, None)
        .await
        .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    let n = rows
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0);
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_body_requires_both_fields_and_refuses_extra_ones() {
        assert!(
            serde_json::from_str::<SettingsBody>(r#"{"timeZone":"UTC","weekStart":"monday"}"#)
                .is_ok()
        );
        for bad in [
            r#"{"timeZone":"UTC"}"#,
            r#"{"weekStart":"monday"}"#,
            r#"{"timeZone":"UTC","weekStart":"monday","x":1}"#,
            r#"{"timeZone":1,"weekStart":"monday"}"#,
            "[]",
        ] {
            assert!(serde_json::from_str::<SettingsBody>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_response_names_the_zone_the_first_day_and_whether_it_was_saved() {
        let ctx = TimeContext::new("Europe/Berlin", WeekStart::Sunday).unwrap();
        assert_eq!(
            body_json(&ctx, &FormatSettings::default(), true),
            json!({
                "timeZone": "Europe/Berlin", "weekStart": "sunday", "saved": true,
                "numberLocale": "en-US", "companyPalette": [],
                "fonts": ["inter", "ibm-plex-sans", "source-serif", "jetbrains-mono", "system"],
                "contentSites": [],
            })
        );
        assert_eq!(
            body_json(&TimeContext::default(), &FormatSettings::default(), false)["saved"],
            json!(false)
        );
    }

    fn body(extra: &str) -> SettingsBody {
        serde_json::from_str(&format!(
            r#"{{"timeZone":"UTC","weekStart":"monday"{extra}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn a_body_without_formatting_fields_keeps_what_is_stored() {
        let current = FormatSettings {
            number_locale: "id-ID".to_owned(),
            company_palette: vec!["#112233".to_owned(), "#445566".to_owned()],
            fonts: vec!["inter".to_owned()],
            content_sites: vec!["https://img.example".to_owned()],
        };
        assert_eq!(checked_formatting(&body(""), &current).unwrap(), current);
    }

    #[test]
    fn each_formatting_field_outside_its_list_is_refused_with_a_sentence() {
        let cases = [
            (
                r#","numberLocale":"fr-FR""#,
                "numberLocale must be en-US or id-ID.",
            ),
            (
                r##","companyPalette":["#112233"]"##,
                "The company palette needs 2 to 12 colours, or none.",
            ),
            (
                r##","companyPalette":["#112233","blue"]"##,
                "Every company palette colour must look like #1a2b3c.",
            ),
            (
                r#","fonts":["comic-sans"]"#,
                "fonts must be chosen from the fonts the console ships.",
            ),
        ];
        for (extra, message) in cases {
            let err = checked_formatting(&body(extra), &FormatSettings::default()).unwrap_err();
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m == message),
                "{extra}: {err:?}"
            );
        }
        let wildcard = checked_formatting(
            &body(r#","contentSites":["https://*.example"]"#),
            &FormatSettings::default(),
        )
        .unwrap_err();
        assert!(matches!(&wildcard, ApiError::BadRequest(m) if m == CONTENT_SITES_REFUSED));
        let thirteen: Vec<String> = (0..13).map(|_| "\"#000000\"".to_owned()).collect();
        let extra = format!(r#","companyPalette":[{}]"#, thirteen.join(","));
        assert!(checked_formatting(&body(&extra), &FormatSettings::default()).is_err());
    }

    #[test]
    fn colours_are_stored_lower_case_and_fonts_in_shipped_order_without_duplicates() {
        let got = checked_formatting(
            &body(
                r##","companyPalette":["#AABBCC","#112233"],"fonts":["system","inter","inter"]"##,
            ),
            &FormatSettings::default(),
        )
        .unwrap();
        assert_eq!(got.company_palette, ["#aabbcc", "#112233"]);
        assert_eq!(got.fonts, ["inter", "system"]);
    }

    #[test]
    fn the_formatting_settings_join_the_payload_reporting_object() {
        let mut payload = json!({"reporting": {"timeZone": "UTC"}});
        add_formatting(&mut payload, &FormatSettings::default());
        assert_eq!(payload["reporting"]["timeZone"], json!("UTC"));
        assert_eq!(payload["reporting"]["numberLocale"], json!("en-US"));
        let mut none = json!({});
        add_formatting(&mut none, &FormatSettings::default());
        assert_eq!(none, json!({}));
    }
}
