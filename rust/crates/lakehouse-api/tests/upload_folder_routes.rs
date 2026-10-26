//! `/api/uploads/folders*`, `POST /api/uploads/move` and the folder filters of
//! `GET /api/uploads` at the route level (`DATA-35`, plan
//! `docs/superpowers/plans/2026-10-26-data-35-upload-folders.md`, T2): the real
//! router over a real migrated Postgres and real authenticated sessions.
//!
//! No object store, orchestrator or `ClickHouse` is involved: folders touch
//! Postgres only, so none is mocked. The ingest side (`folderId` on a load) is
//! tested in `tests/upload_routes.rs`, which owns the stand-ins for those.
//!
//! The seeded users are those of `0002_seed_identity.sql`: Bayu (Data Engineer,
//! Meridian Group and Meridian Logistics), Andi (Data Engineer, Meridian Retail).
//! A role without `connector:manage` is refused on every route here by
//! `tests/route_auth.rs`, which pins the table both ways.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::uploads::{self, NewUpload};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

const GROUP: &str = "11111111-1111-4111-8111-000000000001";
const RETAIL: &str = "11111111-1111-4111-8111-000000000002";

/// A string no handler of this crate could produce, injected into a request, so
/// that finding it in a response proves request text is echoed.
const MARKER: &str = "ECHOED-TEXT-7c1e90";

struct Caller {
    cookie: String,
    tenant: &'static str,
}

struct Reply {
    status: StatusCode,
    json: Value,
    text: String,
}

struct Stack {
    app: TestApp,
}

impl Stack {
    async fn start() -> Self {
        Self {
            app: spin_up().await,
        }
    }

    async fn bayu(&self) -> Caller {
        Caller {
            cookie: session_cookie_for_seeded_user(&self.app.pool, "bayu@meridian.example").await,
            tenant: GROUP,
        }
    }

    async fn andi(&self) -> Caller {
        Caller {
            cookie: session_cookie_for_seeded_user(&self.app.pool, "andi@meridian.example").await,
            tenant: RETAIL,
        }
    }

    async fn send(&self, who: &Caller, method: &str, uri: &str, body: Option<Value>) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("cookie", &who.cookie)
            .header("x-tenant", who.tenant);
        let body = match body {
            Some(value) => {
                request = request.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .app
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .expect("router never fails a request outright");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Reply {
            status,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            text: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    /// Make a folder; the answer must be 201. Returns its id.
    async fn folder(&self, who: &Caller, name: &str, parent: Option<&str>) -> String {
        let body = match parent {
            Some(parent) => json!({ "name": name, "parentId": parent }),
            None => json!({ "name": name }),
        };
        let reply = self
            .send(who, "POST", "/api/uploads/folders", Some(body))
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
        reply.json["id"].as_str().unwrap().to_owned()
    }

    /// Put an upload of `tenant` in the store, without the bytes: folders never
    /// read them.
    async fn seed(&self, tenant: &str, id: &str, filename: &str, table: Option<&str>) {
        let tenant_id: Uuid = tenant.parse().unwrap();
        uploads::insert(
            &self.app.pool,
            &NewUpload {
                id,
                original_filename: filename,
                storage_key: &format!("uploads/{tenant_id}/{id}.csv"),
                content_type: "text/csv",
                size_bytes: 1,
                sha256: id,
                uploaded_by: "Seeded Uploader",
                tenant_id,
            },
        )
        .await
        .unwrap();
        if let Some(table) = table {
            sqlx::query("UPDATE file_upload SET bronze_table = $2 WHERE id = $1")
                .bind(id)
                .bind(table)
                .execute(&self.app.pool)
                .await
                .unwrap();
        }
    }

    async fn folder_of(&self, id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT folder_id FROM file_upload WHERE id = $1")
            .bind(id)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }

    async fn audit(&self) -> Vec<(String, Option<String>, Option<String>, Value)> {
        sqlx::query_as(
            "SELECT action, resource_kind, resource_id, args FROM audit_event \
             WHERE action LIKE 'upload.%' ORDER BY at",
        )
        .fetch_all(&self.app.pool)
        .await
        .unwrap()
    }
}

fn error_of(reply: &Reply) -> &str {
    reply.json["error"].as_str().unwrap_or_default()
}

/// Create, list with counts, rename, move and delete, each answered and audited.
#[tokio::test]
async fn a_folder_is_made_listed_renamed_moved_and_deleted_and_each_is_audited() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let sales = stack.folder(&bayu, "Sales", None).await;
    let archive = stack.folder(&bayu, "Archive", Some(&sales)).await;
    stack.seed(GROUP, "up-1", "a.csv", None).await;
    let moved = stack
        .send(
            &bayu,
            "POST",
            "/api/uploads/move",
            Some(json!({ "uploadIds": ["up-1"], "folderId": archive })),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.text);
    assert_eq!(moved.json, json!({ "moved": 1 }));

    let listed = stack.send(&bayu, "GET", "/api/uploads/folders", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let rows = listed.json.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "Archive");
    assert_eq!(rows[0]["parentId"], sales.as_str());
    assert_eq!(
        (&rows[0]["fileCount"], &rows[0]["childCount"]),
        (&json!(1), &json!(0))
    );
    assert_eq!(
        (&rows[1]["fileCount"], &rows[1]["childCount"]),
        (&json!(0), &json!(1))
    );
    for hidden in ["tenantId", "tenant_id"] {
        assert!(!listed.text.contains(hidden), "{hidden}");
    }

    let renamed = stack
        .send(
            &bayu,
            "PATCH",
            &format!("/api/uploads/folders/{archive}"),
            Some(json!({ "name": "Old" })),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.text);
    assert_eq!(renamed.json["name"], "Old");
    let to_top = stack
        .send(
            &bayu,
            "PATCH",
            &format!("/api/uploads/folders/{archive}"),
            Some(json!({ "parentId": null })),
        )
        .await;
    assert_eq!(to_top.status, StatusCode::OK, "{}", to_top.text);
    assert!(to_top.json.get("parentId").is_none_or(Value::is_null));

    // The file is out of the folder, then the folder is deleted.
    stack
        .send(
            &bayu,
            "POST",
            "/api/uploads/move",
            Some(json!({ "uploadIds": ["up-1"], "folderId": null })),
        )
        .await;
    let deleted = stack
        .send(
            &bayu,
            "DELETE",
            &format!("/api/uploads/folders/{archive}"),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    let actions: Vec<String> = stack.audit().await.into_iter().map(|a| a.0).collect();
    assert_eq!(
        actions,
        [
            "upload.folder.create",
            "upload.folder.create",
            "upload.move",
            "upload.folder.rename",
            "upload.folder.move",
            "upload.move",
            "upload.folder.delete",
        ]
    );
}

/// Every rule answers its own fixed sentence and changes nothing.
#[tokio::test]
async fn each_rule_is_refused_with_its_own_sentence() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let a = stack.folder(&bayu, "A", None).await;
    let b = stack.folder(&bayu, "B", Some(&a)).await;
    let c = stack.folder(&bayu, "C", Some(&b)).await;

    let deep = stack
        .send(
            &bayu,
            "POST",
            "/api/uploads/folders",
            Some(json!({ "name": "D", "parentId": c })),
        )
        .await;
    assert_eq!(deep.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        error_of(&deep),
        "Folders can be nested at most 3 levels deep."
    );

    let cycle = stack
        .send(
            &bayu,
            "PATCH",
            &format!("/api/uploads/folders/{a}"),
            Some(json!({ "parentId": c })),
        )
        .await;
    assert_eq!(cycle.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        error_of(&cycle),
        "A folder cannot be moved into itself or into one of its own folders."
    );

    let taken = stack
        .send(
            &bayu,
            "POST",
            "/api/uploads/folders",
            Some(json!({ "name": " a " })),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT);
    assert_eq!(
        error_of(&taken),
        "A folder with that name already exists in the same place."
    );

    for bad in [json!({ "name": "" }), json!({ "name": "x".repeat(101) })] {
        let reply = stack
            .send(&bayu, "POST", "/api/uploads/folders", Some(bad))
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&reply), "A folder name has 1 to 100 characters.");
    }

    let not_empty = stack
        .send(&bayu, "DELETE", &format!("/api/uploads/folders/{a}"), None)
        .await;
    assert_eq!(not_empty.status, StatusCode::CONFLICT);
    assert_eq!(
        error_of(&not_empty),
        "This folder still holds files or folders. Move them out and delete its folders first."
    );
    stack.seed(GROUP, "up-1", "a.csv", None).await;
    stack
        .send(
            &bayu,
            "POST",
            "/api/uploads/move",
            Some(json!({ "uploadIds": ["up-1"], "folderId": c })),
        )
        .await;
    let with_file = stack
        .send(&bayu, "DELETE", &format!("/api/uploads/folders/{c}"), None)
        .await;
    assert_eq!(with_file.status, StatusCode::CONFLICT, "{}", with_file.text);

    let listed = stack.send(&bayu, "GET", "/api/uploads/folders", None).await;
    assert_eq!(listed.json.as_array().unwrap().len(), 3, "nothing changed");
}

/// A body that is not what the route reads is a 400 with a sentence of ours, and
/// the text sent is not echoed (principle 4).
#[tokio::test]
async fn a_bad_body_is_a_fixed_sentence_that_echoes_nothing() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let folder = stack.folder(&bayu, "A", None).await;
    let patch = format!("/api/uploads/folders/{folder}");
    for (method, uri, body, sentence) in [
        (
            "POST",
            "/api/uploads/folders",
            json!([MARKER]),
            "The request body must be a JSON object.",
        ),
        (
            "POST",
            "/api/uploads/folders",
            json!({ "name": 3 }),
            "name must be text.",
        ),
        (
            "POST",
            "/api/uploads/folders",
            json!({ "name": "x", "parentId": 3 }),
            "parentId must be text or null.",
        ),
        (
            "PATCH",
            patch.as_str(),
            json!({}),
            "Send a name, a parentId, or both.",
        ),
        (
            "PATCH",
            patch.as_str(),
            json!({ "name": [MARKER] }),
            "name must be text.",
        ),
        (
            "POST",
            "/api/uploads/move",
            json!({ "folderId": null }),
            "uploadIds must be a list of 1 to 500 upload ids.",
        ),
        (
            "POST",
            "/api/uploads/move",
            json!({ "uploadIds": [], "folderId": null }),
            "uploadIds must be a list of 1 to 500 upload ids.",
        ),
        (
            "POST",
            "/api/uploads/move",
            json!({ "uploadIds": ["up-1"] }),
            "folderId must be text or null.",
        ),
        (
            "POST",
            "/api/uploads/move",
            json!({ "uploadIds": [MARKER], "folderId": 3 }),
            "folderId must be text or null.",
        ),
    ] {
        let reply = stack.send(&bayu, method, uri, Some(body)).await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{method} {uri}: {}",
            reply.text
        );
        assert_eq!(error_of(&reply), sentence, "{method} {uri}");
        assert!(!reply.text.contains(MARKER), "{method} {uri}");
    }
}

/// Another tenant's folder answers like one that does not exist, on every folder
/// route, and its files are not movable by a stranger; nothing changes.
#[tokio::test]
async fn another_tenants_folder_and_files_are_not_found_and_nothing_moves() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    let mine = stack.folder(&bayu, "Mine", None).await;
    stack.seed(GROUP, "up-g", "g.csv", None).await;
    stack.seed(RETAIL, "up-r", "r.csv", None).await;
    let unknown = stack
        .send(
            &andi,
            "DELETE",
            "/api/uploads/folders/uf-does-not-exist",
            None,
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&unknown), "Folder not found.");

    for (method, uri, body) in [
        (
            "PATCH",
            format!("/api/uploads/folders/{mine}"),
            Some(json!({ "name": "Stolen" })),
        ),
        ("DELETE", format!("/api/uploads/folders/{mine}"), None),
        (
            "POST",
            "/api/uploads/folders".to_owned(),
            Some(json!({ "name": "Child", "parentId": mine })),
        ),
        (
            "POST",
            "/api/uploads/move".to_owned(),
            Some(json!({ "uploadIds": ["up-r"], "folderId": mine })),
        ),
    ] {
        let reply = stack.send(&andi, method, &uri, body).await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{method} {uri}: {}",
            reply.text
        );
        assert_eq!(
            reply.json, unknown.json,
            "{method} {uri}: no oracle for which ids exist"
        );
    }

    // A file of another tenant: the upload sentence, and the caller's own file in
    // the same request does not move either.
    stack.folder(&andi, "Theirs", None).await;
    let theirs = stack
        .send(&andi, "GET", "/api/uploads/folders", None)
        .await
        .json[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mixed = stack
        .send(
            &andi,
            "POST",
            "/api/uploads/move",
            Some(json!({ "uploadIds": ["up-r", "up-g"], "folderId": theirs })),
        )
        .await;
    assert_eq!(mixed.status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&mixed), "Upload not found.");
    assert_eq!(stack.folder_of("up-r").await, None, "all or nothing");
    assert_eq!(stack.folder_of("up-g").await, None);

    // Each tenant lists only its own.
    let retail = stack.send(&andi, "GET", "/api/uploads/folders", None).await;
    assert_eq!(retail.json.as_array().unwrap().len(), 1);
    assert_eq!(retail.json[0]["name"], "Theirs");
    let group = stack.send(&bayu, "GET", "/api/uploads/folders", None).await;
    assert_eq!(group.json.as_array().unwrap().len(), 1);
    assert_eq!(group.json[0]["name"], "Mine");
}

/// Plan section 4, item 2: `/api/uploads/folders` and `/api/uploads/move` are not
/// caught by `/api/uploads/{id}`. The per-upload routes answer "Upload not found."
/// for an id they do not know; these answer in their own words.
#[tokio::test]
async fn the_static_paths_are_not_taken_by_the_per_upload_routes() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;

    let list = stack.send(&bayu, "GET", "/api/uploads/folders", None).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    assert_eq!(list.json, json!([]), "a list of folders, not an upload");

    let delete = stack
        .send(&bayu, "DELETE", "/api/uploads/folders/uf-nothing", None)
        .await;
    assert_eq!(
        error_of(&delete),
        "Folder not found.",
        "not the upload route"
    );

    let moved = stack
        .send(
            &bayu,
            "POST",
            "/api/uploads/move",
            Some(json!({ "uploadIds": ["up-nothing"], "folderId": null })),
        )
        .await;
    assert_eq!(moved.status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&moved), "Upload not found.");

    // The upload routes still answer for upload ids.
    let upload = stack
        .send(&bayu, "GET", "/api/uploads/up-nothing", None)
        .await;
    assert_eq!(error_of(&upload), "Upload not found.");
}

/// `GET /api/uploads` narrows by folder and by text before the window is cut, and
/// each upload carries its `folderId`; without a filter it is the list it was.
#[tokio::test]
async fn the_upload_list_can_be_narrowed_by_folder_and_text() {
    let stack = Stack::start().await;
    let bayu = stack.bayu().await;
    let andi = stack.andi().await;
    let folder = stack.folder(&bayu, "Q1", None).await;
    stack
        .seed(GROUP, "up-1", "stock_jan.csv", Some("stock_raw"))
        .await;
    stack
        .seed(GROUP, "up-2", "orders.csv", Some("orders_raw"))
        .await;
    stack.seed(GROUP, "up-3", "Stock_feb.csv", None).await;
    stack.seed(RETAIL, "up-r", "stock_retail.csv", None).await;
    stack
        .send(
            &bayu,
            "POST",
            "/api/uploads/move",
            Some(json!({ "uploadIds": ["up-1"], "folderId": folder })),
        )
        .await;

    let ids = |reply: &Reply| -> Vec<String> {
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        reply
            .json
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let all = stack.send(&bayu, "GET", "/api/uploads", None).await;
    assert_eq!(ids(&all), ["up-3", "up-2", "up-1"]);
    let by_id = |id: &str| {
        all.json
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(by_id("up-1")["folderId"], folder.as_str());
    assert!(by_id("up-2").get("folderId").is_none(), "no folder, no key");

    let in_folder = stack
        .send(
            &bayu,
            "GET",
            &format!("/api/uploads?folderId={folder}"),
            None,
        )
        .await;
    assert_eq!(ids(&in_folder), ["up-1"]);
    let none = stack
        .send(&bayu, "GET", "/api/uploads?folderId=none", None)
        .await;
    assert_eq!(ids(&none), ["up-3", "up-2"]);
    let text = stack.send(&bayu, "GET", "/api/uploads?q=STOCK", None).await;
    assert_eq!(ids(&text), ["up-3", "up-1"], "every folder, any case");
    let table = stack
        .send(&bayu, "GET", "/api/uploads?q=orders_raw", None)
        .await;
    assert_eq!(ids(&table), ["up-2"], "a table name matches too");
    let both = stack
        .send(&bayu, "GET", "/api/uploads?folderId=none&q=stock", None)
        .await;
    assert_eq!(ids(&both), ["up-3"]);
    let wild = stack.send(&bayu, "GET", "/api/uploads?q=%25", None).await;
    assert!(ids(&wild).is_empty(), "% matches itself, not everything");

    // Another tenant's folder id lists nothing of this one's files.
    let foreign = stack
        .send(
            &andi,
            "GET",
            &format!("/api/uploads?folderId={folder}"),
            None,
        )
        .await;
    assert!(ids(&foreign).is_empty());
}
