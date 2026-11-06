//! `GET`/`PUT /api/settings/reporting` — the deployment's report time zone
//! and first day of the week (`BI-9`), and the number locale, company palette
//! and allowed fonts (`BI-17` part A, migration `0086`), and the sites image
//! and web cards may load from (`BI-16` part B, migration `0087`).
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
//! Two lists are in the answer only for a caller with `settings:write` (`PR
//! #204` review (SHOULD-FIX) 3): the roles that may not download
//! (`downloadDeniedRoles`) and the outside domains a report may go to
//! (`reportAllowedDomains`). They name the deployment's roles and its
//! partners, which every signed-in person has no business reading. Everything
//! else — the content sites among it — is everybody's.
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
use lakehouse_store::PgPool;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::runtime_settings::{EffectiveEmail, Source};
use crate::state::AppState;
use crate::upstream_error;

/// What the person is told when `contentSites` is not a list of sites.
const CONTENT_SITES_REFUSED: &str = "contentSites must be at most 50 sites, each written \
https://host or https://host:port, with no wildcard, path or user name.";

/// What the person is told when `downloadDeniedRoles` names a role that does
/// not exist.
const DOWNLOAD_ROLES_REFUSED: &str = "downloadDeniedRoles must name roles that exist.";

/// What the person is told when `reportAllowedDomains` is not at most 50
/// domain names (`RPT-1`, D8).
const REPORT_DOMAINS_REFUSED: &str = "reportAllowedDomains must be at most 50 domain names such as example.com, with no wildcard, address or path.";

/// What the person is told when `resultCache` is not a freshness rule or its
/// minutes are out of range (`BI-4` part B, feature decision 2).
const RESULT_CACHE_REFUSED: &str = "resultCache must be {\"mode\": \"default\"} or {\"mode\": \"fresh\"}, or {\"mode\": \"minutes\", \"minutes\": a whole number from 1 to 1440}.";

/// What the person is told when their roles may not download (`BI-1`, D7).
pub(crate) const DOWNLOADS_OFF: &str = "Downloads are switched off for your role.";

/// What the person is told when the body is not the two fields.
const BODY_SHAPE: &str = "body must be {\"timeZone\": string, \"weekStart\": \"monday\"|\"sunday\", \
\"numberLocale\"?: \"en-US\"|\"id-ID\", \"companyPalette\"?: [\"#rrggbb\", ...], \"fonts\"?: [string, ...], \"contentSites\"?: [\"https://host\", ...], \"downloadDeniedRoles\"?: [string, ...], \"reportAllowedDomains\"?: [string, ...], \"publicLinksEnabled\"?: boolean, \"resultCache\"?: {\"mode\": \"default\"|\"fresh\"|\"minutes\", \"minutes\"?: number}}";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsBody {
    time_zone: String,
    week_start: String,
    number_locale: Option<String>,
    company_palette: Option<Vec<String>>,
    fonts: Option<Vec<String>>,
    content_sites: Option<Vec<String>>,
    download_denied_roles: Option<Vec<String>>,
    report_allowed_domains: Option<Vec<String>>,
    public_links_enabled: Option<bool>,
    /// `BI-4` part B: absent keeps the stored rule. Read as raw JSON so a
    /// malformed rule gets [`RESULT_CACHE_REFUSED`], not the body-shape text.
    result_cache: Option<Value>,
}

/// The formatting half of the settings, as the console reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormatSettings {
    pub number_locale: String,
    pub company_palette: Vec<String>,
    pub fonts: Vec<String>,
    pub content_sites: Vec<String>,
    /// Role names that may not download (`BI-1`, D7). Not part of
    /// [`Self::to_json`]: the payload every view reads (a public link and a
    /// signed embed included) must not list the deployment's role names; the
    /// settings screen reads it through `body_json`, and the dashboard page
    /// gets only the answer for its own viewer (`canDownload`).
    pub download_denied_roles: Vec<String>,
    /// The outside email domains a scheduled report may go to (`RPT-1`, D8).
    /// Like `download_denied_roles`, not part of [`Self::to_json`].
    pub report_allowed_domains: Vec<String>,
    /// Whether public links work (`BI-19` part A, feature A7). Not part of
    /// [`Self::to_json`]: an anonymous reader has no use for it, and an
    /// administrator reads it through `body_json`.
    pub public_links_enabled: bool,
    /// The install's freshness rule for remembered dashboard results (`BI-4`
    /// part B). Not part of [`Self::to_json`]: the dashboard payload carries
    /// the board's effective rule as `cacheRule`; the settings screen reads
    /// this one through `body_json`.
    pub result_cache: lakehouse_bi::cache_rule::CacheRule,
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
            download_denied_roles: Vec::new(),
            report_allowed_domains: Vec::new(),
            public_links_enabled: true,
            result_cache: lakehouse_bi::cache_rule::CacheRule::Follow,
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
            download_denied_roles: row.download_denied_roles.clone(),
            report_allowed_domains: row.report_allowed_domains.clone(),
            public_links_enabled: row.public_links_enabled,
            // A stored pair the CHECKs should have refused reads as "always
            // fresh" (fail closed), never as the default.
            result_cache: lakehouse_bi::cache_rule::CacheRule::parse(
                &row.result_cache_mode,
                row.result_cache_minutes.map(i64::from),
            )
            .unwrap_or(lakehouse_bi::cache_rule::CacheRule::Fresh),
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

/// The content sites of a board's image and web cards: the deployment's list,
/// in the order it was saved, narrowed to the origins those cards load from
/// (their address as `https://host[:port]`; every other kind has none).
///
/// `PR #233 review SHOULD-FIX 1`: the embed payload (`embed::render_board_payload`)
/// and the frame answer (`embed::board_content_sites`) both narrow with this
/// one function over the same cards (`embed::board_cards`), so the policy can
/// neither refuse a card the payload shows nor allow a site no card uses. The
/// whole list would tell whoever reads a shared link which sites the
/// deployment allows (`PR #199 review M SHOULD-FIX BI-16 1`, decision M6), and
/// a card whose site was taken off the list is not in it any more, so the card
/// says so, as it does in the console.
pub(crate) fn content_sites_of_cards(
    cards: &[&lakehouse_bi::store::StoredChartSpec],
    format: &FormatSettings,
) -> Vec<String> {
    let mut origins: Vec<String> = Vec::new();
    for card in cards {
        // The stored definition of an image or web card carries its address;
        // every other kind has none.
        let Some(url) = card.def.types.url.as_deref() else {
            continue;
        };
        if let Some(origin) = lakehouse_bi::embed_access::https_origin_of(url)
            && !origins.contains(&origin)
        {
            origins.push(origin);
        }
    }
    format
        .content_sites
        .iter()
        .filter(|site| origins.contains(site))
        .cloned()
        .collect()
}

/// Add the formatting settings to an anonymous payload (a public link or a
/// signed embed): the number locale, the company palette and the fonts as
/// everywhere, and the content sites of that dashboard's own image and web
/// cards, already narrowed by [`content_sites_of_cards`].
pub(crate) fn add_formatting_for_anonymous(
    body: &mut Value,
    format: &FormatSettings,
    content_sites: &[String],
) {
    add_formatting(body, format);
    let Some(reporting) = body.get_mut("reporting").and_then(Value::as_object_mut) else {
        return;
    };
    reporting.insert("contentSites".to_owned(), json!(content_sites));
}

/// Check the optional formatting fields of a body against the closed lists,
/// with the sentence the person is told.
fn checked_formatting(
    parsed: &SettingsBody,
    current: &FormatSettings,
    known_roles: Option<&[String]>,
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
    let download_denied_roles = match (&parsed.download_denied_roles, known_roles) {
        (Some(roles), Some(known)) => {
            // `BI-1` D7: only names of roles that exist, so a typo does not
            // look like a rule. A fixed sentence (the names are the
            // caller's own text).
            if roles.iter().all(|r| known.contains(r)) {
                let mut roles = roles.clone();
                roles.sort();
                roles.dedup();
                roles
            } else {
                return Err(ApiError::BadRequest(DOWNLOAD_ROLES_REFUSED.to_owned()));
            }
        }
        _ => current.download_denied_roles.clone(),
    };
    let report_allowed_domains = match &parsed.report_allowed_domains {
        // A fixed sentence: the entries are the administrator's own text.
        Some(domains) => validate_report_domains(domains)
            .ok_or_else(|| ApiError::BadRequest(REPORT_DOMAINS_REFUSED.to_owned()))?,
        None => current.report_allowed_domains.clone(),
    };
    Ok(FormatSettings {
        number_locale,
        company_palette,
        fonts,
        content_sites,
        download_denied_roles,
        report_allowed_domains,
        public_links_enabled: parsed
            .public_links_enabled
            .unwrap_or(current.public_links_enabled),
        result_cache: match &parsed.result_cache {
            // A fixed sentence: the rule is the caller's own text.
            Some(raw) => serde_json::from_value(raw.clone())
                .map_err(|_| ApiError::BadRequest(RESULT_CACHE_REFUSED.to_owned()))?,
            None => current.result_cache,
        },
    })
}

/// The outside email domains for reports (`RPT-1`, D8): at most 50, each a
/// valid domain name (`report_runner::domain_is_valid`: no wildcard, at least
/// two labels), lower-cased, sorted, no duplicates. `None` when any entry or the
/// count is refused.
fn validate_report_domains(domains: &[String]) -> Option<Vec<String>> {
    if domains.len() > 50 {
        return None;
    }
    let mut out = Vec::with_capacity(domains.len());
    for raw in domains {
        let domain = raw.trim().to_ascii_lowercase();
        if !crate::routes::report_runner::domain_is_valid(&domain) {
            return None;
        }
        out.push(domain);
    }
    out.sort();
    out.dedup();
    Some(out)
}

/// Whether `principal` may download (`BI-1`, D7): a Platform Admin always,
/// and when no role is switched off everyone; otherwise a person who holds
/// at least one role that is not on the list. The one copy: the PDF route,
/// the chart and table exports, the dashboard payload's `canDownload` and the
/// assistant's tools all call it (rule 4).
#[must_use]
pub(crate) fn may_download(principal: &Principal, denied_roles: &[String]) -> bool {
    if denied_roles.is_empty()
        || principal
            .role_names
            .iter()
            .any(|r| r == PLATFORM_ADMIN_ROLE)
    {
        return true;
    }
    principal
        .role_names
        .iter()
        .any(|role| !denied_roles.contains(role))
}

/// The role that always may download, whatever the list says.
const PLATFORM_ADMIN_ROLE: &str = "Platform Admin";

/// Refuse with [`DOWNLOADS_OFF`] unless `principal` may download.
///
/// # Errors
///
/// 403 when every role of the person is switched off; the settings read
/// failing is reported through `upstream_error`.
pub(crate) async fn require_download(
    state: &AppState,
    principal: &Principal,
) -> Result<(), ApiError> {
    let settings = format_settings(state).await?;
    if may_download(principal, &settings.download_denied_roles) {
        Ok(())
    } else {
        Err(ApiError::PermissionDenied(DOWNLOADS_OFF.to_owned()))
    }
}

/// Today's date in the report time zone, `YYYY-MM-DD`, for a download's file
/// name (`BI-1` review note: it was the UTC date). The engine knows the zone
/// list, so it is asked (`toDate(now(zone))`, the same expression "today" is
/// everywhere else); when it cannot answer, the UTC date stands and the failure
/// is logged under a reference.
pub(crate) async fn report_date(state: &AppState) -> String {
    let utc = || chrono::Utc::now().format("%Y-%m-%d").to_string();
    let Ok(time) = time_context(state).await else {
        return utc();
    };
    let sql = format!("SELECT toString({}) AS d", time.today_expr());
    match state.clickhouse.rows(&sql, None).await {
        Ok(rows) => rows
            .first()
            .and_then(|r| r.get("d"))
            .and_then(Value::as_str)
            .filter(|d| d.len() == 10 && d.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
            .map_or_else(utc, str::to_owned),
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            utc()
        }
    }
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
    let ctx = base_time_context(state).await?;
    // BI-16 part C3: the uploaded maps `RegionOf` may name ride in the context
    // every formula compile receives. A map list that cannot be read leaves
    // none (logged): `RegionOf` then says it finds no such map.
    let Some(pool) = state.pg.as_deref() else {
        return Ok(ctx);
    };
    let maps = match lakehouse_store::geo_map::list(pool).await {
        Ok(rows) => rows
            .into_iter()
            .map(|m| lakehouse_bi::geo::SpatialMap {
                id: m.id,
                name: m.name,
                region_lookup: m.region_lookup,
            })
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "the uploaded maps could not be listed for the formula compiler");
            Vec::new()
        }
    };
    Ok(ctx.with_maps(maps))
}

/// [`time_context`] without the uploaded maps.
async fn base_time_context(state: &AppState) -> Result<TimeContext, ApiError> {
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
        .map(|ctx| {
            ctx.with_content_sites(saved.content_sites.clone())
                // BI-17 part A, decision 6: the company palette rides in the
                // context every chart save already receives, so the
                // assistant's `create_chart` can draw a new chart with it.
                .with_company_palette(saved.company_palette.clone())
        })
        .ok_or_else(|| ApiError::Internal("The reporting settings are invalid.".to_owned()))
}

/// The settings answer. `administrator` says whether the caller may read the
/// two lists only an administrator reads (the roles that may not download and
/// the outside domains a report may go to); they are left out of the answer
/// for anyone else (`PR #204` review (SHOULD-FIX) 3).
fn body_json(
    ctx: &TimeContext,
    format: &FormatSettings,
    saved: bool,
    administrator: bool,
) -> Value {
    let mut body = json!({
        "timeZone": ctx.zone(),
        "weekStart": ctx.week_start().as_str(),
        "saved": saved,
        // BI-19 part A and BI-4 part B: the public-links switch and the
        // result-cache rule are read by everyone who signs in (the first by
        // the public-link pages, the second by every dashboard read), so
        // they are in the answer for every caller.
        "publicLinksEnabled": format.public_links_enabled,
        "resultCache": format.result_cache.to_json(),
    });
    // PR #204 review (SHOULD-FIX) 3: the two lists only an administrator
    // reads are in the answer only for a caller with `settings:write`.
    if administrator {
        body["downloadDeniedRoles"] = json!(format.download_denied_roles);
        body["reportAllowedDomains"] = json!(format.report_allowed_domains);
    }
    if let (Some(map), Value::Object(fields)) = (body.as_object_mut(), format.to_json()) {
        map.extend(fields);
    }
    body
}

/// `GET /api/settings/reporting` — the zone and first day in use, and whether
/// they were ever saved (`saved: false` means these are the defaults).
///
/// The two lists only an administrator reads — the roles that may not download
/// and the outside domains a report may go to — are in the answer only for a
/// caller with `settings:write` (`PR #204` review (SHOULD-FIX) 3): they name
/// the deployment's roles and its partners, which every signed-in person has
/// no business reading. Everything else, the content sites among it, is
/// everybody's (`SEC-47`'s Content-Security-Policy reads this answer from the
/// console server for every signed-in page).
///
/// # Errors
///
/// 5xx from [`time_context`] when the settings cannot be read.
pub async fn get_reporting(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let ctx = time_context(&state).await?;
    let format = format_settings(&state).await?;
    let saved = match state.pg.as_deref() {
        Some(pool) => lakehouse_store::reporting_settings::get(pool)
            .await
            .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?
            .is_some(),
        None => false,
    };
    Ok(ApiJson(body_json(
        &ctx,
        &format,
        saved,
        principal.has("settings:write"),
    )))
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
    // Role names are read only when the body sends the list.
    let known_roles = if parsed.download_denied_roles.is_some() {
        Some(
            lakehouse_store::identity::list_roles(pool)
                .await
                .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?
                .into_iter()
                .map(|r| r.name)
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let format = checked_formatting(&parsed, &current, known_roles.as_deref())?;
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
            download_denied_roles: format.download_denied_roles,
            report_allowed_domains: format.report_allowed_domains,
            public_links_enabled: format.public_links_enabled,
            result_cache_mode: format.result_cache.mode().to_owned(),
            result_cache_minutes: format
                .result_cache
                .minutes()
                .and_then(|m| i32::try_from(m).ok()),
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
        // The caller may save, so they may read the two lists back.
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

// ── ADM-7: the deployment's email settings ───────────────────────────────

/// What the person is told when the body is not the email fields.
const EMAIL_BODY_SHAPE: &str = "body must be {\"host\"?: string|null, \"port\"?: number|null, \
\"security\"?: \"tls\"|\"starttls\"|null, \"user\"?: string|null, \"password\"?: string|null, \
\"from\"?: string|null, \"consolePublicUrl\"?: string|null}";

/// What the person is told when the body sends a field the server's
/// environment owns (plan D1). Nothing is written.
const SET_BY_THE_SERVER: &str =
    "This setting is set in the server's environment and cannot be changed here.";

/// What the person is told when the mail server's name is not a host name.
const HOST_REFUSED: &str = "host must be a mail server name, such as smtp.example.com.";

/// What the person is told when the port is not a port number.
const PORT_REFUSED: &str = "port must be a whole number from 1 to 65535.";

/// What the person is told when the security mode is not one of the two.
const SECURITY_REFUSED: &str = "security must be tls or starttls.";

/// What the person is told when the user name is not text.
const USER_REFUSED: &str = "user must be text.";

/// What the person is told when the sender address is not an address.
const FROM_REFUSED: &str = "from must be an email address, such as reports@example.com.";

/// What the person is told when the console's public address is not an
/// http(s) address.
const CONSOLE_URL_REFUSED: &str = "consolePublicUrl must start with http:// or https://.";

/// What the person is told when the caller's account has no address a test
/// email could go to.
const NO_ACCOUNT_ADDRESS: &str = "This account has no email address, so no test email can be sent.";

/// What the person is told when they ask for too many test emails.
const TEST_EMAIL_TOO_SOON: &str = "Wait a moment before asking for another test email.";

/// What the person is told when the installation key that seals the password
/// is not configured.
const NO_INSTALLATION_KEY: &str =
    "The installation key is not configured, so a password cannot be saved.";

/// The subject of the test email (plan D10).
const TEST_EMAIL_SUBJECT: &str = "Test email from RantAI Lakehouse";

/// The fields the body of a save may carry (plan D9). A field that is not
/// there keeps what is stored; a field that is there and `null` clears it.
const EMAIL_FIELDS: [&str; 7] = [
    "host",
    "port",
    "security",
    "user",
    "password",
    "from",
    "consolePublicUrl",
];

/// The body of a save, read as a map of the fields it carries.
///
/// A map, not a struct: a struct with `Option` fields cannot tell a field
/// that was not sent from a field that was sent as `null`, and the two mean
/// different things (keep the stored value, or clear it — plan D5).
fn parse_email_body(body: &Bytes) -> Result<serde_json::Map<String, Value>, ApiError> {
    let parsed: Value = serde_json::from_slice(body)
        .map_err(|_| ApiError::BadRequest(EMAIL_BODY_SHAPE.to_owned()))?;
    let Value::Object(fields) = parsed else {
        return Err(ApiError::BadRequest(EMAIL_BODY_SHAPE.to_owned()));
    };
    for key in fields.keys() {
        if !EMAIL_FIELDS.contains(&key.as_str()) {
            return Err(ApiError::BadRequest(EMAIL_BODY_SHAPE.to_owned()));
        }
    }
    Ok(fields)
}

/// The string a field carries, or `None` when it carries an explicit `null`
/// (which clears the setting). `refused` is the fixed sentence the person
/// is told when the value is not text (plan D9).
fn string_field(refused: &str, value: &Value) -> Result<Option<String>, ApiError> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => Err(ApiError::BadRequest(refused.to_owned())),
    }
}

/// One field of the answer: its effective value and where it came from
/// (plan D1).
fn field_json(value: impl Into<Value>, source: Source) -> Value {
    json!({ "value": value.into(), "source": source.as_str() })
}

/// The email settings answer: every field's effective value and its source.
///
/// The password is never in it — only whether one is set (plan D5). The
/// console shows a field the server owns as read-only, and this is where it
/// learns that.
fn email_settings_json(effective: &EffectiveEmail) -> Value {
    json!({
        "host": field_json(effective.host.clone(), effective.host_source),
        "port": field_json(effective.port, effective.port_source),
        "security": field_json(effective.security.as_str(), effective.security_source),
        "user": field_json(effective.user.clone(), effective.user_source),
        "password": {
            "set": effective.password_set,
            "source": effective.password_source.as_str(),
        },
        "from": field_json(effective.from.clone(), effective.from_source),
        "consolePublicUrl": field_json(
            effective.console_public_url.clone(),
            effective.console_public_url_source,
        ),
    })
}

/// `GET /api/settings/email` — the effective email settings, and where each
/// value comes from (plan D1).
///
/// Reading needs `settings:write` like changing does (plan D4): a mail
/// server's name and account are not for every signed-in person.
///
/// # Errors
///
/// 5xx through `upstream_error` when the saved row cannot be read: the
/// answer must not claim a value is "not set" when the database could not
/// be asked.
pub async fn get_email(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let effective = crate::runtime_settings::email_or_fail(&state)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    Ok(ApiJson(email_settings_json(&effective)))
}

/// The new value of a text field, and whether the stored value moves.
///
/// `sent` is `None` when the body did not send the field (keep what is
/// stored), `Some(Value::Null)` when it sent an explicit null (clear it),
/// and a value when it sent one. `check` validates the text and returns the
/// value to store, or `None` when the text is not usable (which is refused
/// with `refused`, the fixed sentence that names the field).
fn text_field(
    sent: Option<&Value>,
    old: Option<&str>,
    refused: &str,
    check: impl Fn(&str) -> Option<String>,
) -> Result<(Option<String>, bool), ApiError> {
    let Some(value) = sent else {
        return Ok((old.map(str::to_owned), false));
    };
    let Some(text) = string_field(refused, value)? else {
        return Ok((None, old.is_some()));
    };
    let checked = check(&text).ok_or_else(|| ApiError::BadRequest(refused.to_owned()))?;
    let changed = old != Some(checked.as_str());
    Ok((Some(checked), changed))
}

/// The new port, and whether the stored port moves.
fn port_field(sent: Option<&Value>, old: Option<i32>) -> Result<(Option<i32>, bool), ApiError> {
    let Some(value) = sent else {
        return Ok((old, false));
    };
    if value.is_null() {
        return Ok((None, old.is_some()));
    }
    let number = value
        .as_i64()
        .ok_or_else(|| ApiError::BadRequest(PORT_REFUSED.to_owned()))?;
    let number = u16::try_from(number)
        .ok()
        .filter(|port| *port > 0)
        .map(i32::from)
        .ok_or_else(|| ApiError::BadRequest(PORT_REFUSED.to_owned()))?;
    Ok((Some(number), old != Some(number)))
}

/// The new sealed password, and whether the stored password moves.
///
/// The plaintext is sealed before it is stored and never reaches a row, a
/// log or a response (plan D5). The sealed bytes are never compared (the
/// seal is randomised), so a password that is sent always counts as
/// changed.
fn password_field(
    state: &AppState,
    sent: Option<&Value>,
    old: Option<&[u8]>,
) -> Result<(Option<Vec<u8>>, bool), ApiError> {
    let Some(value) = sent else {
        return Ok((old.map(<[u8]>::to_vec), false));
    };
    let Some(plain) = string_field(USER_REFUSED, value)? else {
        return Ok((None, old.is_some()));
    };
    let keyring = state
        .connector_secret_store
        .keyring()
        .map_err(|_| ApiError::Unavailable(NO_INSTALLATION_KEY.to_owned()))?;
    let sealed = keyring
        .seal(
            crate::runtime_settings::EMAIL_PASSWORD_SEAL_NAME,
            plain.as_bytes(),
        )
        .map_err(|_| ApiError::Unavailable(NO_INSTALLATION_KEY.to_owned()))?;
    Ok((Some(sealed), true))
}

/// The row a save writes, and the names of the fields it changes (plan D6).
///
/// Every field follows the same rule (plan D5): not sent keeps what is
/// stored, an explicit `null` clears it, a value replaces it.
fn email_row_to_save(
    state: &AppState,
    parsed: &serde_json::Map<String, Value>,
    current: Option<&lakehouse_store::email_settings::EmailSettings>,
) -> Result<
    (
        lakehouse_store::email_settings::EmailSettings,
        Vec<&'static str>,
    ),
    ApiError,
> {
    let old_host = current.and_then(|c| c.smtp_host.as_deref());
    let old_port = current.and_then(|c| c.smtp_port);
    let old_security = current.and_then(|c| c.smtp_security.as_deref());
    let old_user = current.and_then(|c| c.smtp_user.as_deref());
    let old_password = current.and_then(|c| c.smtp_pass.as_deref());
    let old_from = current.and_then(|c| c.smtp_from.as_deref());
    let old_url = current.and_then(|c| c.console_public_url.as_deref());

    let (host, host_changed) = text_field(parsed.get("host"), old_host, HOST_REFUSED, |text| {
        let text = text.trim();
        // The workspace's one hostname rule
        // (`lakehouse_store::ingest_spec::validate_hostname`).
        lakehouse_store::ingest_spec::validate_hostname("host", text)
            .ok()
            .map(|()| text.to_owned())
    })?;
    let (port, port_changed) = port_field(parsed.get("port"), old_port)?;
    let (security, security_changed) = text_field(
        parsed.get("security"),
        old_security,
        SECURITY_REFUSED,
        |text| {
            let text = text.trim().to_ascii_lowercase();
            crate::runtime_settings::EmailSecurity::parse(&text).map(|_| text)
        },
    )?;
    let (user, user_changed) = text_field(parsed.get("user"), old_user, USER_REFUSED, |text| {
        // An empty name means "no account": the sender then sends no
        // credentials at all.
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    })?;
    let (password, password_changed) = password_field(state, parsed.get("password"), old_password)?;
    let (from, from_changed) = text_field(parsed.get("from"), old_from, FROM_REFUSED, |text| {
        // The workspace's one address rule
        // (`routes::report_runner::normalise_address`).
        crate::routes::report_runner::normalise_address(text)
    })?;
    let (console_public_url, url_changed) = text_field(
        parsed.get("consolePublicUrl"),
        old_url,
        CONSOLE_URL_REFUSED,
        |text| {
            let text = text.trim().trim_end_matches('/');
            (text.starts_with("https://") || text.starts_with("http://")).then(|| text.to_owned())
        },
    )?;

    let mut changed: Vec<&'static str> = Vec::new();
    if host_changed {
        changed.push("host");
    }
    if port_changed {
        changed.push("port");
    }
    if security_changed {
        changed.push("security");
    }
    if user_changed {
        changed.push("user");
    }
    if password_changed {
        changed.push("password");
    }
    if from_changed {
        changed.push("from");
    }
    if url_changed {
        changed.push("consolePublicUrl");
    }
    Ok((
        lakehouse_store::email_settings::EmailSettings {
            smtp_host: host,
            smtp_port: port,
            smtp_security: security,
            smtp_user: user,
            smtp_pass: password,
            smtp_from: from,
            console_public_url,
        },
        changed,
    ))
}

/// One feature of the deployment, as the console shows it (plan D8).
fn feature(name: &str, on: bool, missing: &[&str], source: &str) -> Value {
    json!({
        "feature": name,
        "on": on,
        "missing": missing,
        "where": source,
    })
}

/// Which of the deployment's features are set up, and what is missing for
/// the ones that are not (plan D8).
///
/// It reports CONFIGURATION, never health: "on" means the settings are in
/// place, which is what the console can honestly say. Whether the mail
/// server really answers is the test email's question.
///
/// A missing setting is named the way the operator sets it: the environment
/// variable for a setting only the server owns, the console's own field
/// name for a setting the console can set. `where` says which of the two
/// it is — a feature the server owns is read-only in the console.
fn deployment_features(state: &AppState, email: &EffectiveEmail) -> Vec<Value> {
    let config = &state.config;
    // PDF export needs all three of these (feature decision 6).
    let mut pdf_missing: Vec<&str> = Vec::new();
    if config.gotenberg_url.is_none() {
        pdf_missing.push("GOTENBERG_URL");
    }
    if config.console_internal_url.is_none() {
        pdf_missing.push("CONSOLE_INTERNAL_URL");
    }
    if config.render_token_secret.is_none() {
        pdf_missing.push("RENDER_TOKEN_SECRET");
    }
    // Single sign-on is on when the `OIDC` authenticator was built, which
    // needs an issuer and a client id (`state.rs`'s `oidc_config`).
    let single_sign_on = state
        .auth
        .as_ref()
        .and_then(|auth| auth.oidc.as_ref())
        .is_some();
    let sso_missing: Vec<&str> = if single_sign_on {
        Vec::new()
    } else {
        vec!["OIDC_ISSUER", "OIDC_CLIENT_ID"]
    };
    // The assistant is on when it has a key (`ADM-9` reads the saved one
    // in slice 2; today the environment owns it).
    let assistant = !config.llm_key.is_empty();
    let assistant_missing: Vec<&str> = if assistant {
        Vec::new()
    } else {
        vec!["LLM_KEY"]
    };
    vec![
        feature("pdfExport", pdf_missing.is_empty(), &pdf_missing, "server"),
        feature(
            "email",
            email.host.is_some(),
            if email.host.is_some() { &[] } else { &["host"] },
            if email.host_source == Source::Server {
                "server"
            } else {
                "console"
            },
        ),
        feature(
            "consoleAddress",
            email.console_public_url.is_some(),
            if email.console_public_url.is_some() {
                &[]
            } else {
                &["consolePublicUrl"]
            },
            if email.console_public_url_source == Source::Server {
                "server"
            } else {
                "console"
            },
        ),
        feature("singleSignOn", single_sign_on, &sso_missing, "server"),
        feature("assistant", assistant, &assistant_missing, "server"),
    ]
}

/// `GET /api/settings/deployment` — which of the deployment's features are
/// set up, and what is missing for the ones that are not (plan D8).
///
/// It reports configuration, never health: a feature is "on" when its
/// settings are in place. Backups are not in the list — the API cannot see
/// the backup container's settings (feature page, Limits).
///
/// # Errors
///
/// 5xx through `upstream_error` when the saved email settings cannot be
/// read: the answer must not claim a feature is off when the database
/// could not be asked.
pub async fn get_deployment(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let email = crate::runtime_settings::email_or_fail(&state)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    Ok(ApiJson(json!({
        "features": deployment_features(&state, &email),
    })))
}

/// `PUT /api/settings/email` — save the mail server, the account, the
/// password, the `From` address and the console's public address (plan D9).
///
/// A field that is not sent keeps what is stored; a field sent as `null` is
/// cleared; a field sent with a value replaces what is stored (plan D5).
/// A field the server's environment owns is refused with a 409 and nothing
/// is written (plan D1).
///
/// # Errors
///
/// 400 with a fixed sentence when the body is not the email fields or a
/// value is not usable; 409 when the environment owns a field the body
/// sends; 503 without a Postgres pool or without the installation key;
/// 5xx through `upstream_error` when the database fails.
pub async fn put_email(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    // Parsed by hand so a malformed body gets this module's message.
    let parsed = parse_email_body(&body)?;
    let Some(pool) = state.pg.as_deref() else {
        return Err(ApiError::Unavailable(
            "Settings need a database, and none is configured.".to_owned(),
        )
        .into());
    };
    // The effective values, to know which fields the environment owns.
    let effective = crate::runtime_settings::email_or_fail(&state)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    for (sent, source) in [
        (parsed.contains_key("host"), effective.host_source),
        (parsed.contains_key("port"), effective.port_source),
        (parsed.contains_key("security"), effective.security_source),
        (parsed.contains_key("user"), effective.user_source),
        (parsed.contains_key("password"), effective.password_source),
        (parsed.contains_key("from"), effective.from_source),
        (
            parsed.contains_key("consolePublicUrl"),
            effective.console_public_url_source,
        ),
    ] {
        if sent && source == Source::Server {
            return Err(ApiError::Conflict(SET_BY_THE_SERVER.to_owned()).into());
        }
    }

    let current = lakehouse_store::email_settings::get(pool)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    let (row, changed) = email_row_to_save(&state, &parsed, current.as_ref())?;

    lakehouse_store::email_settings::upsert(pool, &row, &principal.id.uuid().to_string())
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    // The save is visible to the next send on this process at once (D3).
    state.email_settings_cache.clear().await;
    // One audit row for the save, naming the fields that changed (D6).
    audit_email_update(pool, &principal, &changed).await;

    let effective = crate::runtime_settings::email_or_fail(&state)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    Ok(ApiJson(email_settings_json(&effective)))
}

/// One audit row for a save of the email settings (plan D6): the action and
/// the NAMES of the fields that changed, never a value and never the
/// password.
///
/// Best effort, like every other audit write: a failed audit write never
/// fails, or undoes, the save it records.
async fn audit_email_update(pool: &PgPool, principal: &Principal, fields: &[&str]) {
    let event = NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: "settings.email.update".to_owned(),
        resource_kind: Some("settings".to_owned()),
        resource_id: Some("email".to_owned()),
        args: Some(json!({ "fields": fields })),
        outcome: "executed".to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    };
    if let Err(err) = store_audit::insert(pool, event).await {
        tracing::warn!(%err, "failed to record the email settings audit event");
    }
}

/// `POST /api/settings/email/test` — send one short email to the caller's
/// own account address, with the effective settings (plan D10).
///
/// The body carries no address: the recipient is the caller's own account
/// address, so the button cannot be used to send mail to anyone else. At
/// most one per caller every ten seconds (a 429 with a `Retry-After`).
///
/// The answer is one of the fixed sentences of plan D7, mapped from
/// `lakehouse_notify`'s failure kinds — never from the mail server's own
/// words.
///
/// # Errors
///
/// 400 when the caller's account has no address a test email could go to;
/// 429 when the caller asked too soon; 503 without a Postgres pool; 5xx
/// through `upstream_error` when the database fails.
pub async fn post_email_test(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    if let Err(wait) = state
        .email_test_limiter
        .claim(principal.id.uuid(), std::time::Instant::now())
        .await
    {
        return Err(ApiError::TooManyRequests {
            message: TEST_EMAIL_TOO_SOON.to_owned(),
            retry_after_secs: wait,
        }
        .into());
    }
    let Some(pool) = state.pg.as_deref() else {
        return Err(ApiError::Unavailable(
            "Settings need a database, and none is configured.".to_owned(),
        )
        .into());
    };
    // The recipient: the caller's own account address, and nothing else.
    let Some(user_id) = principal.user_id() else {
        return Err(ApiError::BadRequest(NO_ACCOUNT_ADDRESS.to_owned()).into());
    };
    let user = lakehouse_store::identity::get_user(pool, &user_id.to_string())
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    let Some(address) = crate::routes::report_runner::normalise_address(&user.email) else {
        return Err(ApiError::BadRequest(NO_ACCOUNT_ADDRESS.to_owned()).into());
    };
    // The effective settings: what a send would use, saved or from the
    // environment. The body of this request is never read.
    let effective = crate::runtime_settings::email_or_fail(&state)
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::STORE, &err))?;
    let sender = lakehouse_notify::EmailSender::new(effective.smtp());
    let html = test_email_html(effective.console_public_url.as_deref());
    match sender
        .send_classified(&address, TEST_EMAIL_SUBJECT, &html)
        .await
    {
        Ok(()) => Ok(ApiJson(json!({
            "sent": true,
            "message": format!("Sent to {address}."),
        }))),
        Err(failure) => Ok(ApiJson(json!({
            "sent": false,
            "message": test_email_sentence(failure),
        }))),
    }
}

/// The one sentence for each way a test email can fail (plan D7). The
/// `lakehouse_notify` kind decides the sentence; the mail server's own
/// words never reach it.
fn test_email_sentence(failure: lakehouse_notify::DeliveryFailure) -> &'static str {
    match failure {
        lakehouse_notify::DeliveryFailure::EmailNotConfigured => {
            "Email is not set up: the mail server is missing."
        }
        // The webhook kinds cannot happen on an email send. They share the
        // one sentence so that a kind this feature does not know still
        // answers with a fixed sentence (plan D7), never with a new one.
        lakehouse_notify::DeliveryFailure::MailServerUnreachable
        | lakehouse_notify::DeliveryFailure::WebhookUnreachable
        | lakehouse_notify::DeliveryFailure::WebhookAddressRefused
        | lakehouse_notify::DeliveryFailure::WebhookServerError
        | lakehouse_notify::DeliveryFailure::WebhookRedirected
        | lakehouse_notify::DeliveryFailure::ChannelUnsupported => {
            "Could not reach the mail server."
        }
        lakehouse_notify::DeliveryFailure::MailServerRefused => {
            "The mail server refused the user name or password."
        }
        lakehouse_notify::DeliveryFailure::MailServerTlsFailed => {
            "The secure connection to the mail server failed."
        }
        // The address was refused before the message left: the account has
        // no address this deployment can send to.
        lakehouse_notify::DeliveryFailure::EmailAddressRefused => NO_ACCOUNT_ADDRESS,
    }
}

/// The body of the test email: short, and carrying the console's public
/// address when one is set.
fn test_email_html(console_public_url: Option<&str>) -> String {
    let link = match console_public_url {
        Some(url) => format!("<p>Open the console at <a href=\"{url}\">{url}</a>.</p>"),
        None => "<p>Open the console to see your dashboards.</p>".to_owned(),
    };
    format!(
        "<div style=\"font-family:system-ui,sans-serif\"><p>This is a test email from your RantAI Lakehouse console.</p>{link}</div>"
    )
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
            body_json(&ctx, &FormatSettings::default(), true, true),
            json!({
                "timeZone": "Europe/Berlin", "weekStart": "sunday", "saved": true,
                "numberLocale": "en-US", "companyPalette": [],
                "fonts": ["inter", "ibm-plex-sans", "source-serif", "jetbrains-mono", "system"],
                "contentSites": [], "downloadDeniedRoles": [], "reportAllowedDomains": [], "publicLinksEnabled": true,
                "resultCache": {"mode": "default"},
            })
        );
        assert_eq!(
            body_json(
                &TimeContext::default(),
                &FormatSettings::default(),
                false,
                true
            )["saved"],
            json!(false)
        );
    }

    /// `PR #204` review (SHOULD-FIX) 3: the two lists only an administrator
    /// reads are left out of the answer for anyone else.
    #[test]
    fn the_two_administrator_lists_are_left_out_for_anyone_else() {
        let denied = FormatSettings {
            download_denied_roles: vec!["Viewer".to_owned()],
            report_allowed_domains: vec!["partner.example".to_owned()],
            ..FormatSettings::default()
        };
        let body = body_json(&TimeContext::default(), &denied, true, false);
        assert!(body.get("downloadDeniedRoles").is_none(), "{body}");
        assert!(body.get("reportAllowedDomains").is_none(), "{body}");
        // Everything else is everybody's.
        assert_eq!(body["contentSites"], json!([]));
        assert_eq!(body["numberLocale"], json!("en-US"));
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
            download_denied_roles: vec!["Viewer".to_owned()],
            report_allowed_domains: vec!["example.com".to_owned()],
            public_links_enabled: false,
            result_cache: lakehouse_bi::cache_rule::CacheRule::Minutes(45),
        };
        assert_eq!(
            checked_formatting(&body(""), &current, None).unwrap(),
            current
        );
    }

    /// `BI-19` part A (feature A7): the switch is kept when the body does not
    /// send it, changed when it does, and is for the settings screen only.
    #[test]
    fn the_public_links_switch_is_kept_changed_and_kept_out_of_the_dashboard_payload() {
        let on = FormatSettings::default();
        assert!(on.public_links_enabled, "on at first, as before");
        let off = checked_formatting(&body(r#","publicLinksEnabled":false"#), &on, None).unwrap();
        assert!(!off.public_links_enabled);
        assert!(
            !checked_formatting(&body(""), &off, None)
                .unwrap()
                .public_links_enabled
        );
        assert!(
            checked_formatting(&body(r#","publicLinksEnabled":true"#), &off, None)
                .unwrap()
                .public_links_enabled
        );
        assert!(off.to_json().get("publicLinksEnabled").is_none());
        // `false`: the switch is everybody's (only the two lists of `PR #204`
        // review SHOULD-FIX 3 are an administrator's), so it must be in the
        // answer for a caller without `settings:write` too.
        assert_eq!(
            body_json(&TimeContext::default(), &off, true, false)["publicLinksEnabled"],
            json!(false)
        );
        // `true`: an administrator reads the same switch.
        assert_eq!(
            body_json(&TimeContext::default(), &off, true, true)["publicLinksEnabled"],
            json!(false)
        );
    }

    /// `BI-4` part B T3: the install's freshness rule is kept when the body
    /// does not send it, changed when it does, and refused with one sentence
    /// when its minutes are out of range or it is not a rule.
    #[test]
    fn the_result_cache_rule_is_kept_changed_and_refused_with_a_sentence() {
        use lakehouse_bi::cache_rule::CacheRule;
        let first = FormatSettings::default();
        assert_eq!(first.result_cache, CacheRule::Follow);
        let set = checked_formatting(
            &body(r#","resultCache":{"mode":"minutes","minutes":30}"#),
            &first,
            None,
        )
        .unwrap();
        assert_eq!(set.result_cache, CacheRule::Minutes(30));
        assert_eq!(
            checked_formatting(&body(""), &set, None)
                .unwrap()
                .result_cache,
            CacheRule::Minutes(30),
            "a body without the rule keeps it"
        );
        // `false`: the freshness rule is everybody's (only the two lists of
        // `PR #204` review SHOULD-FIX 3 are an administrator's), so it must
        // be in the answer for a caller without `settings:write` too.
        assert_eq!(
            body_json(&TimeContext::default(), &set, true, false)["resultCache"],
            json!({"mode": "minutes", "minutes": 30})
        );
        for bad in [
            r#","resultCache":{"mode":"minutes","minutes":0}"#,
            r#","resultCache":{"mode":"minutes","minutes":1441}"#,
            r#","resultCache":{"mode":"minutes"}"#,
            r#","resultCache":{"mode":"fresh","minutes":5}"#,
            r#","resultCache":{"mode":"sometimes"}"#,
            r#","resultCache":"fresh""#,
        ] {
            let err = checked_formatting(&body(bad), &set, None).unwrap_err();
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m == RESULT_CACHE_REFUSED),
                "{bad}"
            );
        }
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
            let err =
                checked_formatting(&body(extra), &FormatSettings::default(), None).unwrap_err();
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m == message),
                "{extra}: {err:?}"
            );
        }
        let wildcard = checked_formatting(
            &body(r#","contentSites":["https://*.example"]"#),
            &FormatSettings::default(),
            None,
        )
        .unwrap_err();
        assert!(matches!(&wildcard, ApiError::BadRequest(m) if m == CONTENT_SITES_REFUSED));
        let thirteen: Vec<String> = (0..13).map(|_| "\"#000000\"".to_owned()).collect();
        let extra = format!(r#","companyPalette":[{}]"#, thirteen.join(","));
        assert!(checked_formatting(&body(&extra), &FormatSettings::default(), None).is_err());
    }

    #[test]
    fn colours_are_stored_lower_case_and_fonts_in_shipped_order_without_duplicates() {
        let got = checked_formatting(
            &body(
                r##","companyPalette":["#AABBCC","#112233"],"fonts":["system","inter","inter"]"##,
            ),
            &FormatSettings::default(),
            None,
        )
        .unwrap();
        assert_eq!(got.company_palette, ["#aabbcc", "#112233"]);
        assert_eq!(got.fonts, ["inter", "system"]);
    }

    fn person(roles: &[&str]) -> Principal {
        Principal {
            id: lakehouse_auth::PrincipalId::User(uuid::Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "Test".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse("dashboard:read"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: roles.iter().map(|r| (*r).to_owned()).collect(),
        }
    }

    #[test]
    fn nothing_is_switched_off_until_an_administrator_chooses() {
        assert!(may_download(&person(&["Viewer"]), &[]));
        assert!(may_download(&person(&[]), &[]));
    }

    #[test]
    fn a_platform_admin_may_download_whatever_the_list_says() {
        let denied = vec!["Platform Admin".to_owned(), "Viewer".to_owned()];
        assert!(may_download(&person(&["Platform Admin"]), &denied));
    }

    #[test]
    fn one_role_off_does_not_stop_a_person_who_holds_another() {
        let denied = vec!["Viewer".to_owned()];
        assert!(may_download(&person(&["Viewer", "Analyst"]), &denied));
        assert!(!may_download(&person(&["Viewer"]), &denied));
    }

    #[test]
    fn a_person_whose_every_role_is_off_cannot_download() {
        let denied = vec!["Viewer".to_owned(), "Analyst".to_owned()];
        assert!(!may_download(&person(&["Viewer", "Analyst"]), &denied));
        assert!(
            !may_download(&person(&[]), &denied),
            "no role is not a way around the list"
        );
    }

    #[test]
    fn denied_roles_must_exist_and_are_sorted_without_duplicates() {
        let known = vec!["Analyst".to_owned(), "Viewer".to_owned()];
        let got = checked_formatting(
            &body(r#","downloadDeniedRoles":["Viewer","Analyst","Viewer"]"#),
            &FormatSettings::default(),
            Some(&known),
        )
        .unwrap();
        assert_eq!(got.download_denied_roles, ["Analyst", "Viewer"]);
        let err = checked_formatting(
            &body(r#","downloadDeniedRoles":["Nobody"]"#),
            &FormatSettings::default(),
            Some(&known),
        )
        .unwrap_err();
        assert!(matches!(&err, ApiError::BadRequest(m) if m == DOWNLOAD_ROLES_REFUSED));
    }

    #[test]
    fn the_payload_reporting_object_never_lists_role_names() {
        let denied = FormatSettings {
            download_denied_roles: vec!["Viewer".to_owned()],
            ..FormatSettings::default()
        };
        assert!(denied.to_json().get("downloadDeniedRoles").is_none());
        assert_eq!(
            body_json(&TimeContext::default(), &denied, true, true)["downloadDeniedRoles"],
            json!(["Viewer"])
        );
    }

    /// A stored `kind` chart on `board`, with `def` as saved.
    fn card(
        kind: &str,
        board: &str,
        def: serde_json::Value,
    ) -> lakehouse_bi::store::StoredChartSpec {
        lakehouse_bi::store::StoredChartSpec {
            spec: serde_json::from_value(json!({
                "id": "c", "title": "C", "kind": kind, "mart": "mart_a",
                "sql": "", "x": "", "y": "visitors"
            }))
            .unwrap(),
            source: lakehouse_bi::specs::ChartSource::Ui,
            board: board.to_owned(),
            def: serde_json::from_value(def).unwrap(),
            has_year: false,
            created_by: None,
            created_at: None,
        }
    }

    /// A stored image card on `board` whose address is `url`.
    fn image_card(board: &str, url: &str) -> lakehouse_bi::store::StoredChartSpec {
        card(
            "image",
            board,
            json!({
                "title": "C", "mart": "mart_a", "kind": "image",
                "measures": ["visitors"], "url": url
            }),
        )
    }

    /// `PR #199 review M SHOULD-FIX BI-16 1` (decision M6) and `PR #233
    /// review SHOULD-FIX 1`: an anonymous payload carries only the origins of
    /// that dashboard's own image and web cards, and only the ones an
    /// administrator still allows.
    #[test]
    fn the_content_sites_are_the_ones_the_boards_cards_load_from() {
        let img = image_card("b1", "https://img.example/logo.png");
        let status = image_card("b1", "https://status.example/board");
        // A bar chart has no address of its own.
        let bar = card(
            "bar",
            "b1",
            json!({
                "title": "B", "mart": "mart_a", "kind": "bar",
                "dimension": "x", "measures": ["visitors"]
            }),
        );
        let format = FormatSettings {
            content_sites: vec![
                "https://img.example".to_owned(),
                "https://status.example".to_owned(),
                "https://other.example".to_owned(),
            ],
            ..FormatSettings::default()
        };
        assert_eq!(
            content_sites_of_cards(&[&img, &status, &bar], &format),
            ["https://img.example", "https://status.example"]
        );
        // A card whose site was taken off the list is not in it, so the card
        // says the site is no longer allowed.
        let mut without = format.clone();
        without.content_sites = vec!["https://other.example".to_owned()];
        assert_eq!(
            content_sites_of_cards(&[&img], &without),
            [] as [std::string::String; 0]
        );
        assert_eq!(
            content_sites_of_cards(&[], &format),
            [] as [std::string::String; 0]
        );
    }

    #[test]
    fn an_anonymous_payload_carries_the_content_sites_it_is_given() {
        let mut payload = json!({ "reporting": { "timeZone": "UTC" } });
        let format = FormatSettings {
            content_sites: vec![
                "https://img.example".to_owned(),
                "https://other.example".to_owned(),
            ],
            ..FormatSettings::default()
        };
        add_formatting_for_anonymous(&mut payload, &format, &["https://img.example".to_owned()]);
        assert_eq!(
            payload["reporting"]["contentSites"],
            json!(["https://img.example"])
        );
        // The rest of the formatting rides along as everywhere.
        assert_eq!(payload["reporting"]["numberLocale"], json!("en-US"));

        let mut none = json!({ "reporting": { "timeZone": "UTC" } });
        add_formatting_for_anonymous(&mut none, &format, &[]);
        assert_eq!(none["reporting"]["contentSites"], json!([]));
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

    /// `RPT-1` D8: the outside domains are validated names, lower-cased, sorted,
    /// without duplicates, at most 50, and refused with one fixed sentence.
    #[test]
    fn report_domains_are_validated_lower_cased_and_bounded() {
        let current = FormatSettings::default();
        let got = checked_formatting(
            &body(r#","reportAllowedDomains":["Partner.Example "," a.example","partner.example"]"#),
            &current,
            None,
        )
        .unwrap();
        assert_eq!(got.report_allowed_domains, ["a.example", "partner.example"]);
        for bad in [
            r#"["*.example.com"]"#,
            r#"["example"]"#,
            r#"["a@example.com"]"#,
            r#"["https://example.com"]"#,
            r#"[""]"#,
        ] {
            let err = checked_formatting(
                &body(&format!(r#","reportAllowedDomains":{bad}"#)),
                &current,
                None,
            )
            .unwrap_err();
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m == REPORT_DOMAINS_REFUSED),
                "{bad}"
            );
        }
        let fifty_one: Vec<String> = (0..51).map(|i| format!("d{i}.example")).collect();
        assert!(validate_report_domains(&fifty_one).is_none());
        assert!(validate_report_domains(&fifty_one[..50]).is_some());
        // Absent: what is stored stays.
        let kept = FormatSettings {
            report_allowed_domains: vec!["x.example".to_owned()],
            ..FormatSettings::default()
        };
        assert_eq!(
            checked_formatting(&body(""), &kept, None)
                .unwrap()
                .report_allowed_domains,
            ["x.example"]
        );
    }

    #[test]
    fn the_domain_list_is_in_the_settings_screen_and_not_in_the_dashboard_payload() {
        let f = FormatSettings {
            report_allowed_domains: vec!["x.example".to_owned()],
            ..FormatSettings::default()
        };
        assert!(f.to_json().get("reportAllowedDomains").is_none());
        assert_eq!(
            body_json(&TimeContext::default(), &f, true, true)["reportAllowedDomains"],
            json!(["x.example"])
        );
    }
}
