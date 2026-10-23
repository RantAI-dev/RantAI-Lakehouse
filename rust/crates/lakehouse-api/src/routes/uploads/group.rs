//! Several files into one table (`DATA-17` slice 5, plan AD4; feature page
//! `upload-big-files-and-more-formats.md`, decisions 4 and 5).
//!
//! A load of several files is ONE ingest call, ONE Dagster run and ONE table. The
//! call names a primary upload (the one in the path) and up to nine more
//! (`alsoUploads`); each stays its own `file_upload` row, listed, deleted and
//! duplicate-checked as before, and the others carry `loaded_with` = the primary
//! (`0092_upload_loaded_with.sql`). There is no batch object.
//!
//! # What "the same" means (decision 4)
//!
//! - **One kind of file.** All the same format and wrapper (`job_kind`), and all
//!   converted the same way: a workbook is never loaded with a CSV. A zip's files
//!   count as one upload and must agree among themselves, as for a single zip.
//! - **The same columns, by name.** Columns are matched by the names the load gives
//!   them (`clean_names`, the job's own cleaning), and their ORDER may differ: the job
//!   reads every file in the first file's order. A file whose names differ is listed
//!   with the columns it lacks (`missing`) and the ones the first file lacks
//!   (`extra`), and Load stays off until it is removed.
//! - **Types that fit.** The first file's types decide: its column list when the
//!   caller sends one (the types a person confirmed), otherwise the starting guess.
//!   A column of another file whose sampled values do not fit that type is a
//!   `typeConflicts` entry, with both types, but only for a column the FIRST file's
//!   own sample fits: where the first file has values that do not fit either (a
//!   Parquet type whose text is wider than the type, or a type a person chose over
//!   such values and told the load to empty them), the second file is no worse and
//!   `onBadValue` decides, as it does for one file. A load with no column list is every
//!   column text, which every value fits, so there is nothing to conflict. This is
//!   judged on the sample the preview reads (the first 10,000 rows), as the guess is:
//!   a value further down is what `onBadValue` is for.
//!
//! # The same list, twice
//!
//! `POST /api/uploads/{id}/compare` answers the list so the Check step can show it
//! before Load, and `ingest` computes it again (a file may have changed, and the API
//! never trusts that the console asked) and refuses with it ([`refusal`]).
//!
//! # Posture
//!
//! A file of `alsoUploads` is looked up like the one in the path: the same 404 for a
//! file that is unknown, of another tenant or of none (no oracle for ids), and it must
//! be of the PRIMARY's tenant (`tenant_id` equality, then `upload_in_tenants` on the
//! caller's own tenants), so one load never mixes tenants. The cap counts all the
//! files of a load together (decision 1).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::uploads::{self, Upload};
use serde::{Deserialize, Serialize};

use super::columns::{self, Sample};
use super::{
    ALREADY_LOADING, Conversion, MAX_UNPACKED_BYTES, PreviewQuery, STILL_RECEIVING, Settler,
    ZIP_COLUMNS_DIFFER, conversion_of, job_kind, layout_unless_converted, not_found, pool,
    preview_with_sample,
};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upload_formats::Layout;
use crate::upload_types::{ColumnSpec, ColumnType, check_rows, clean_names, validate_specs};

/// The most files one load takes, the first included (decision 5).
pub(super) const MAX_FILES: usize = 10;

const ALSO_RULE: &str =
    "alsoUploads must be a list of at most 9 upload ids, each used once, none of them this upload.";
const COMPARE_BODY: &str = "The request body must be an object with alsoUploads, a list of upload ids, and optionally encoding, delimiter, headerRow, sheet and columns.";
const OTHER_NOT_FOUND: &str = "One of the other files was not found.";
const OTHER_KIND: &str = "All the files of one load must be the same kind of file, for example all CSV or all Parquet. Remove the ones that differ.";
/// The files of one load added together, against [`MAX_UNPACKED_BYTES`] (2 GiB): its own
/// text, not the 50 MB sentence of one `POST /api/uploads` file (plan T6.2 item 5).
const TOO_LARGE_TOGETHER: &str = "The files together are larger than the 2 GB limit.";

/// How many files and names a refusal sentence lists before it says "and N more".
const LISTED: usize = 5;

/// What a file is for the purpose of "the same kind": whether it is converted and,
/// if so, how, and what the job is told it is (`job_kind`).
pub(super) type Kind = (Option<Conversion>, (&'static str, &'static str));

/// The kind of the file `job` reads, as `ingest` and `compare` judge it.
pub(super) fn kind_of(conversion: Option<Conversion>, layout: Option<Layout>) -> Kind {
    (conversion, job_kind(conversion, layout))
}

/// One more file of a load, with how it is read.
pub(super) struct Member {
    pub(super) row: Upload,
    pub(super) conversion: Option<Conversion>,
}

/// Check a list of `alsoUploads` ids that does not need the database: at most 9,
/// each used once, none empty.
///
/// # Errors
///
/// 400 with [`ALSO_RULE`].
pub(super) fn check_also_list(ids: &[String]) -> Result<(), ApiError> {
    let mut seen = std::collections::HashSet::new();
    let fine = ids.len() < MAX_FILES
        && ids
            .iter()
            .all(|id| !id.is_empty() && id.len() <= 128 && seen.insert(id.as_str()));
    if fine {
        Ok(())
    } else {
        Err(ApiError::BadRequest(ALSO_RULE.to_owned()))
    }
}

/// The files of `ids`, settled and checked to load with `primary`: each one exists, is
/// of the primary's tenant (and one of the caller's), is neither being received nor
/// loading, is of the same `kind`, and the files together are under the cap.
///
/// # Errors
///
/// 400 for the primary named among them, a different kind of file or a total over the
/// cap; 404 for a file that is unknown or not this tenant's (one sentence for both);
/// 409 for a file still being received or already loading; 503 when Postgres or the
/// object store is unavailable.
pub(super) async fn members(
    state: &AppState,
    pool: &PgPool,
    principal: &Principal,
    primary: &Upload,
    ids: &[String],
    kind: Kind,
) -> Result<Vec<Member>, ApiError> {
    let mut settler = Settler::new(state, pool);
    let mut total = primary.size_bytes;
    let mut found = Vec::with_capacity(ids.len());
    for id in ids {
        if *id == primary.id {
            return Err(ApiError::BadRequest(ALSO_RULE.to_owned()));
        }
        let other_tenant = || ApiError::NotFound(OTHER_NOT_FOUND.to_owned());
        let Some(row) = uploads::get(pool, id).await? else {
            return Err(other_tenant());
        };
        // Same tenant as the primary, which the route layer has already matched to the
        // caller's; then the caller's own list as well, so the rule does not depend on
        // that one comparison alone.
        if row.tenant_id.is_none()
            || row.tenant_id != primary.tenant_id
            || !uploads::upload_in_tenants(pool, id, &principal.tenant_ids).await?
        {
            return Err(other_tenant());
        }
        if row.status == "receiving" {
            return Err(ApiError::Conflict(STILL_RECEIVING.to_owned()));
        }
        let row = settler.settle(row).await;
        if row.status == "ingesting" {
            return Err(ApiError::Conflict(ALREADY_LOADING.to_owned()));
        }
        let conversion = conversion_of(state, &row).await?;
        let layout = layout_unless_converted(state, &row, conversion).await?;
        if kind_of(conversion, layout) != kind {
            return Err(ApiError::BadRequest(OTHER_KIND.to_owned()));
        }
        total = total.saturating_add(row.size_bytes);
        found.push(Member { row, conversion });
    }
    // Plan T6.2 item 2: the files of one load add up to at most the unpacked cap
    // (`MAX_UPLOAD_BYTES` is the size of one `POST /api/uploads` file).
    if total > i64::try_from(MAX_UNPACKED_BYTES).unwrap_or(i64::MAX) {
        return Err(ApiError::BadRequest(TOO_LARGE_TOGETHER.to_owned()));
    }
    Ok(found)
}

// ── the comparison ───────────────────────────────────────────────────────

/// A column whose values in another file do not fit the type the first file gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypeConflict {
    /// The column, by the name the load gives it.
    column: String,
    /// The first file's type.
    expected: ColumnType,
    /// The type the other file's own values guess (or, for Parquet and Avro, declare).
    found: ColumnType,
}

/// A file that cannot load with the first one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mismatch {
    /// The upload.
    upload_id: String,
    /// Its name, as the console shows it.
    file: String,
    /// Columns the first file has and this one lacks.
    missing: Vec<String>,
    /// Columns this one has and the first file lacks.
    extra: Vec<String>,
    /// Columns both have, whose values here do not fit the first file's type.
    type_conflicts: Vec<TypeConflict>,
}

impl Mismatch {
    fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty() && self.type_conflicts.is_empty()
    }
}

/// How the types of the first file are decided.
pub(super) enum Typing<'a> {
    /// A load with no column list: every column text, and every value fits.
    Text,
    /// The starting guess of the preview.
    Guess,
    /// The column list a person confirmed, in the first file's column order.
    Plan(&'a [ColumnSpec]),
}

/// What differs between the first file's sample and another's. Pure.
///
/// `first_specs` is empty for [`Typing::Text`] and otherwise one entry per column of
/// `first`, in its order; `other_specs` is the same for `other` (its own types, for the
/// `found` side of a conflict).
fn compare_samples(
    first: &Sample,
    first_specs: &[ColumnSpec],
    other: &Sample,
    other_specs: &[ColumnSpec],
) -> (Vec<String>, Vec<String>, Vec<TypeConflict>) {
    let first_names = clean_names(&first.header);
    let other_names = clean_names(&other.header);
    let missing: Vec<String> = first_names
        .iter()
        .filter(|name| !other_names.contains(name))
        .cloned()
        .collect();
    let extra: Vec<String> = other_names
        .iter()
        .filter(|name| !first_names.contains(name))
        .cloned()
        .collect();
    if first_specs.len() != first_names.len() {
        return (missing, extra, Vec::new());
    }
    // The columns both have: the first file's spec for each, named as the load names it,
    // and the other file's values for it, in the first file's order.
    let shared: Vec<(usize, usize)> = first_names
        .iter()
        .enumerate()
        .filter_map(|(i, name)| other_names.iter().position(|n| n == name).map(|j| (i, j)))
        .collect();
    let specs: Vec<ColumnSpec> = shared
        .iter()
        .map(|(i, _)| {
            let mut spec = first_specs[*i].clone();
            spec.name.clone_from(&first_names[*i]);
            spec
        })
        .collect();
    let rows: Vec<Vec<String>> = other
        .rows
        .iter()
        .map(|record| {
            shared
                .iter()
                .map(|(_, j)| record.get(*j).cloned().unwrap_or_default())
                .collect()
        })
        .collect();
    let bad = check_rows(&specs, &rows);
    // The columns the first file's own sample does not fit: no verdict on those.
    let own_rows: Vec<Vec<String>> = first
        .rows
        .iter()
        .map(|record| {
            shared
                .iter()
                .map(|(i, _)| record.get(*i).cloned().unwrap_or_default())
                .collect()
        })
        .collect();
    let first_misfits = check_rows(&specs, &own_rows);
    let conflicts = bad
        .by_column
        .iter()
        .filter(|counted| {
            !first_misfits
                .by_column
                .iter()
                .any(|own| own.column == counted.column)
        })
        .filter_map(|counted| {
            let at = shared
                .iter()
                .position(|(i, _)| first_names[*i] == counted.column)?;
            let (i, j) = shared[at];
            Some(TypeConflict {
                column: counted.column.clone(),
                expected: first_specs[i].column_type,
                found: other_specs
                    .get(j)
                    .map_or(ColumnType::Text, |s| s.column_type),
            })
        })
        .collect();
    (missing, extra, conflicts)
}

/// The files of `members` that cannot load with `first`, in the order given, each with
/// what differs. Reads each file's sample as the preview does, under the reading
/// (`query`) the load will use.
///
/// # Errors
///
/// 400 for a column list that does not fit the first file, or for a file whose own zip
/// has files with other columns; otherwise the preview's errors (503 when the object
/// store is unavailable).
pub(super) async fn mismatches(
    state: &AppState,
    first: &Upload,
    members: &[Member],
    query: &PreviewQuery,
    typing: &Typing<'_>,
) -> Result<Vec<Mismatch>, ApiError> {
    let (_, first_sample) = preview_with_sample(state, first, query).await?;
    let first_specs: Vec<ColumnSpec> = match typing {
        Typing::Text => Vec::new(),
        Typing::Guess => columns::starting_specs(&first_sample),
        Typing::Plan(list) => {
            validate_specs(list, first_sample.header.len()).map_err(ApiError::BadRequest)?;
            list.to_vec()
        }
    };
    let mut found = Vec::new();
    for member in members {
        let (preview, sample) = preview_with_sample(state, &member.row, query).await?;
        if preview
            .archive
            .is_some_and(|archive| archive.files.iter().any(|file| !file.matches_first))
        {
            return Err(ApiError::BadRequest(format!(
                "{}: {ZIP_COLUMNS_DIFFER}",
                member.row.original_filename
            )));
        }
        let own = if first_specs.is_empty() {
            Vec::new()
        } else {
            columns::starting_specs(&sample)
        };
        let (missing, extra, type_conflicts) =
            compare_samples(&first_sample, &first_specs, &sample, &own);
        let mismatch = Mismatch {
            upload_id: member.row.id.clone(),
            file: member.row.original_filename.clone(),
            missing,
            extra,
            type_conflicts,
        };
        if !mismatch.is_empty() {
            found.push(mismatch);
        }
    }
    Ok(found)
}

/// The refusal an `ingest` makes with the list: a 400 whose sentence names the files and
/// what differs in each, cut at a few so a long list stays a sentence.
#[must_use]
pub(super) fn refusal(mismatches: &[Mismatch]) -> ApiError {
    let listed = |names: &[String]| {
        let shown: Vec<&str> = names.iter().take(LISTED).map(String::as_str).collect();
        let more = names.len().saturating_sub(LISTED);
        if more == 0 {
            shown.join(", ")
        } else {
            format!("{} and {more} more", shown.join(", "))
        }
    };
    let files: Vec<String> = mismatches
        .iter()
        .take(LISTED)
        .map(|m| {
            let mut parts = Vec::new();
            if !m.missing.is_empty() {
                parts.push(format!("missing {}", listed(&m.missing)));
            }
            if !m.extra.is_empty() {
                parts.push(format!("extra {}", listed(&m.extra)));
            }
            for conflict in m.type_conflicts.iter().take(LISTED) {
                parts.push(format!(
                    "{} is {:?} in the first file and has values that do not fit",
                    conflict.column, conflict.expected
                ));
            }
            format!("{} ({})", m.file, parts.join("; "))
        })
        .collect();
    let more = mismatches.len().saturating_sub(LISTED);
    let tail = if more == 0 {
        String::new()
    } else {
        format!(" and {more} more")
    };
    ApiError::BadRequest(format!(
        "These files do not have the columns of the first file, so nothing was loaded: {}{tail}. Remove them and load again.",
        files.join("; ")
    ))
}

// ── the route ────────────────────────────────────────────────────────────

/// `POST /api/uploads/{id}/compare`'s body: the other files and, optionally, the
/// reading and the column list the load will use (what the Check step has so far).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompareRequest {
    also_uploads: Vec<String>,
    encoding: Option<String>,
    delimiter: Option<String>,
    header_row: Option<u64>,
    sheet: Option<String>,
    columns: Option<Vec<ColumnSpec>>,
}

/// `POST /api/uploads/{id}/compare`'s response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareResponse {
    /// The files that cannot load with the one in the path, in the order asked; empty
    /// when all can.
    mismatches: Vec<Mismatch>,
}

/// `POST /api/uploads/{id}/compare` — which of `alsoUploads` cannot load into one table
/// with this upload, and why (decision 4). Reads each file's sample; stores nothing.
///
/// The optional `encoding`, `delimiter`, `headerRow` and `sheet` are the reading the
/// load will use, applied to every file (a file read under another delimiter has other
/// columns, which is a mismatch worth showing). The optional `columns` is the first
/// file's column list as the person has it, which decides the types the others must
/// fit; absent, the starting guess does.
///
/// # Errors
///
/// 400 for a body that is not that object, more than 9 files, a repeated file, the
/// path's own upload among them, a different kind of file, files together over the cap,
/// a reading that is not valid or a column list that breaks the rules; 404 for an
/// unknown upload or another tenant's (either side); 409 for a file still being
/// received or loading; 503 when Postgres or the object store is unavailable.
pub async fn compare(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<CompareResponse>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let request: CompareRequest = serde_json::from_slice(&body)
        .map_err(|_err| ApiError::BadRequest(COMPARE_BODY.to_owned()))?;
    check_also_list(&request.also_uploads)?;
    let pool = pool(&state)?;
    let row = uploads::get(pool, &id).await?.ok_or_else(not_found)?;
    if row.status == "receiving" {
        return Err(ApiError::Conflict(STILL_RECEIVING.to_owned()).into());
    }
    let conversion = conversion_of(&state, &row).await?;
    let layout = layout_unless_converted(&state, &row, conversion).await?;
    let others = members(
        &state,
        pool,
        &principal,
        &row,
        &request.also_uploads,
        kind_of(conversion, layout),
    )
    .await?;
    let query = PreviewQuery {
        encoding: request.encoding,
        delimiter: request.delimiter,
        header_row: request.header_row.map(|n| n.to_string()),
        sheet: request.sheet,
        table: None,
    };
    let typing = request
        .columns
        .as_deref()
        .map_or(Typing::Guess, Typing::Plan);
    Ok(ApiJson(CompareResponse {
        mismatches: mismatches(&state, &row, &others, &query, &typing).await?,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::upload_types::DateFormat;

    fn sample(header: &[&str], rows: &[&[&str]]) -> Sample {
        Sample {
            header: header.iter().map(|h| (*h).to_owned()).collect(),
            rows: rows
                .iter()
                .map(|r| r.iter().map(|v| (*v).to_owned()).collect())
                .collect(),
            declared: None,
        }
    }

    fn spec(name: &str, column_type: ColumnType) -> ColumnSpec {
        ColumnSpec {
            source: name.to_owned(),
            name: name.to_owned(),
            column_type,
            date_format: matches!(column_type, ColumnType::Date).then_some(DateFormat::Iso),
            timezone: None,
        }
    }

    #[test]
    fn columns_are_matched_by_cleaned_name_and_their_order_may_differ() {
        let first = sample(&["Id", "Unit Price"], &[&["1", "2"]]);
        let other = sample(&["unit_price", "ID"], &[&["2", "1"]]);
        let (missing, extra, conflicts) = compare_samples(&first, &[], &other, &[]);
        assert!(missing.is_empty() && extra.is_empty() && conflicts.is_empty());
    }

    #[test]
    fn a_file_with_other_columns_lists_what_it_lacks_and_what_it_adds() {
        let first = sample(&["id", "name", "qty"], &[]);
        let other = sample(&["id", "notes", "qty", "region"], &[]);
        let (missing, extra, _) = compare_samples(&first, &[], &other, &[]);
        assert_eq!(missing, ["name"]);
        assert_eq!(extra, ["notes", "region"]);
    }

    #[test]
    fn a_value_that_does_not_fit_the_first_files_type_is_a_conflict_and_text_conflicts_with_nothing()
     {
        let first = sample(&["id", "qty"], &[&["1", "2"]]);
        let first_specs = [
            spec("id", ColumnType::Integer),
            spec("qty", ColumnType::Integer),
        ];
        let other = sample(&["qty", "id"], &[&["5", "7"], &["n/a", "8"]]);
        let other_specs = [
            spec("qty", ColumnType::Text),
            spec("id", ColumnType::Integer),
        ];
        let (missing, extra, conflicts) =
            compare_samples(&first, &first_specs, &other, &other_specs);
        assert!(missing.is_empty() && extra.is_empty());
        assert_eq!(
            conflicts,
            [TypeConflict {
                column: "qty".to_owned(),
                expected: ColumnType::Integer,
                found: ColumnType::Text,
            }]
        );
        // The first file's column is text: whatever the other holds fits.
        let loose = [
            spec("id", ColumnType::Integer),
            spec("qty", ColumnType::Text),
        ];
        assert!(
            compare_samples(&first, &loose, &other, &other_specs)
                .2
                .is_empty()
        );
        // No list at all (every column text): nothing to conflict.
        assert!(compare_samples(&first, &[], &other, &[]).2.is_empty());
    }

    #[test]
    fn a_column_the_first_file_does_not_fit_itself_is_no_conflict_of_another_file() {
        // The same file twice, over a type its own values do not fit (a Parquet type with
        // wider text, or a type a person chose over values they mean to empty).
        let first = sample(&["qty"], &[&["n/a"], &["5"]]);
        let same = sample(&["qty"], &[&["n/a"], &["6"]]);
        let integer = [spec("qty", ColumnType::Integer)];
        let text = [spec("qty", ColumnType::Text)];
        assert!(compare_samples(&first, &integer, &same, &text).2.is_empty());
        // The first file fits, the other does not: that is the conflict.
        let clean = sample(&["qty"], &[&["4"], &["5"]]);
        assert_eq!(compare_samples(&clean, &integer, &same, &text).2.len(), 1);
    }

    #[test]
    fn a_whole_number_fits_a_decimal_column_but_a_decimal_does_not_fit_an_integer_one() {
        let first = sample(&["n"], &[&["1.5"]]);
        let decimal = [spec("n", ColumnType::Decimal)];
        let whole = sample(&["n"], &[&["2"]]);
        assert!(
            compare_samples(&first, &decimal, &whole, &[spec("n", ColumnType::Integer)])
                .2
                .is_empty()
        );
        let integer = [spec("n", ColumnType::Integer)];
        let fractional = sample(&["n"], &[&["2.5"]]);
        let other = [spec("n", ColumnType::Decimal)];
        // PR #186 review BLOCKER 5: the first file's own `1.5` does not fit the `Integer`
        // column it was given, so there is no verdict on that column (the rule that keeps
        // two identical files from ever conflicting); the conflict needs a first file
        // whose own sample fits.
        assert!(
            compare_samples(&first, &integer, &fractional, &other)
                .2
                .is_empty()
        );
        let fits = sample(&["n"], &[&["1"]]);
        let conflicts = compare_samples(&fits, &integer, &fractional, &other).2;
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn the_list_of_other_files_is_at_most_nine_each_used_once() {
        let ids = |n: usize| (0..n).map(|i| format!("up-{i}")).collect::<Vec<_>>();
        assert!(check_also_list(&[]).is_ok());
        assert!(check_also_list(&ids(9)).is_ok());
        assert!(check_also_list(&ids(10)).is_err());
        assert!(check_also_list(&["up-1".to_owned(), "up-1".to_owned()]).is_err());
        assert!(check_also_list(&[String::new()]).is_err());
    }

    #[test]
    fn the_refusal_names_each_file_and_what_differs_and_cuts_a_long_list() {
        let mismatch = |n: usize| Mismatch {
            upload_id: format!("up-{n}"),
            file: format!("file{n}.csv"),
            missing: vec!["a".to_owned()],
            extra: (0..7).map(|i| format!("x{i}")).collect(),
            type_conflicts: Vec::new(),
        };
        let one = refusal(&[mismatch(1)]).to_string();
        assert!(
            one.contains("file1.csv (missing a; extra x0, x1, x2, x3, x4 and 2 more)"),
            "{one}"
        );
        let many: Vec<Mismatch> = (0..8).map(mismatch).collect();
        let long = refusal(&many).to_string();
        assert!(long.contains("and 3 more. Remove them"), "{long}");
        assert!(
            long.contains("file4.csv") && !long.contains("file5.csv"),
            "{long}"
        );
    }
}
