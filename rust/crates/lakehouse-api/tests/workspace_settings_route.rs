//! `GET`/`PUT`/`DELETE /api/settings/general`, the image routes under it and
//! the public `GET /api/branding` (`ADM-2`, slice A).
//!
//! What this proves:
//!
//! * a PNG, a JPEG and a WebP logo are stored and served back with their type,
//!   the headers that keep a stored file from running, and an `ETag`;
//! * a file that breaks a rule is refused with the rule's sentence and
//!   nothing is stored, whatever its name or the part's `Content-Type` say;
//! * the public answer holds the name and each picture's type and version
//!   and nothing else, and needs no session;
//! * the name is trimmed, refused outside 1 to 60 characters or with a
//!   control character, and cleared by an empty string;
//! * reset clears the name and the pictures;
//! * every write leaves one audit row with its action and never the picture.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use common::{
    TestApp, audit_rows, create_principal_with_permissions, create_zero_permission_principal,
    session_cookie_for_user,
};
use serde_json::{Value, json};
use tower::ServiceExt;

const GENERAL: &str = "/api/settings/general";
const BRANDING: &str = "/api/branding";

/// The three sentences of the feature page, written out here and not
/// imported, so a change to the server's text fails the test.
const LOGO_TYPE: &str = "The logo must be a PNG, JPEG or WebP image.";
const LOGO_SIZE: &str = "The logo must be at most 512 KB.";
const ICON_RULE: &str = "The icon must be a PNG image of at most 64 KB.";

const PNG_HEAD: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_HEAD: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0];
const WEBP_HEAD: &[u8] = b"RIFF\x24\x00\x00\x00WEBP";

/// A file that starts with `head` and is `total` bytes long.
fn file(head: &[u8], total: usize) -> Vec<u8> {
    let mut bytes = head.to_vec();
    bytes.resize(total.max(head.len()), 0x41);
    bytes
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }
}

/// A one-part form: the picture under the name `file`, with a file name and a
/// part type chosen by the caller (the server must ignore both).
fn form(file_name: &str, part_type: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let mut body = format!(
        "--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: {part_type}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n--BOUNDARY--\r\n");
    ("multipart/form-data; boundary=BOUNDARY".to_owned(), body)
}

async fn send(
    app: &TestApp,
    cookie: Option<&str>,
    method: &str,
    uri: &str,
    content: Option<(String, Vec<u8>)>,
    extra: &[(&str, &str)],
) -> Reply {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    for (k, v) in extra {
        request = request.header(*k, *v);
    }
    let body = match content {
        Some((content_type, bytes)) => {
            request = request.header("content-type", content_type);
            Body::from(bytes)
        }
        None => Body::empty(),
    };
    let response = app
        .router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body: to_bytes(body, 64 * 1024 * 1024).await.unwrap().to_vec(),
    }
}

async fn admin(app: &TestApp) -> String {
    let id = create_principal_with_permissions(&app.pool, "settings:write").await;
    session_cookie_for_user(&app.pool, id).await
}

async fn put_image(app: &TestApp, cookie: &str, kind: &str, bytes: &[u8]) -> Reply {
    send(
        app,
        Some(cookie),
        "PUT",
        &format!("{GENERAL}/images/{kind}"),
        Some(form("logo.png", "image/png", bytes)),
        &[],
    )
    .await
}

async fn put_name(app: &TestApp, cookie: &str, name: &str) -> Reply {
    send(
        app,
        Some(cookie),
        "PUT",
        GENERAL,
        Some((
            "application/json".to_owned(),
            json!({ "workspaceName": name }).to_string().into_bytes(),
        )),
        &[],
    )
    .await
}

/// How many rows the settings table holds: nothing stored means zero.
async fn stored_rows(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM workspace_settings")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_png_a_jpeg_and_a_webp_logo_are_served_back_with_their_type() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    for (head, media_type) in [
        (PNG_HEAD, "image/png"),
        (JPEG_HEAD, "image/jpeg"),
        (WEBP_HEAD, "image/webp"),
    ] {
        let bytes = file(head, 300);
        // The part claims to be a text file: only the first bytes count.
        let stored = send(
            &app,
            Some(&cookie),
            "PUT",
            &format!("{GENERAL}/images/logo"),
            Some(form("notes.txt", "text/plain", &bytes)),
            &[],
        )
        .await;
        assert_eq!(stored.status, StatusCode::OK, "{media_type}");

        // No session: the sign-in page asks before anybody has one.
        let served = send(
            &app,
            None,
            "GET",
            &format!("{BRANDING}/images/logo"),
            None,
            &[],
        )
        .await;
        assert_eq!(served.status, StatusCode::OK, "{media_type}");
        assert_eq!(served.body, bytes, "{media_type}");
        assert_eq!(served.header("content-type"), media_type);
        assert_eq!(served.header("x-content-type-options"), "nosniff");
        assert_eq!(
            served.header("content-security-policy"),
            "default-src 'none'; sandbox"
        );
        assert_eq!(served.header("content-disposition"), "inline");
        assert_eq!(served.header("cache-control"), "public, max-age=60");
        let etag = served.header("etag").to_owned();
        assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");

        let unchanged = send(
            &app,
            None,
            "GET",
            &format!("{BRANDING}/images/logo"),
            None,
            &[("if-none-match", &etag)],
        )
        .await;
        assert_eq!(unchanged.status, StatusCode::NOT_MODIFIED, "{media_type}");
        assert_eq!(unchanged.body, Vec::<u8>::new(), "{media_type}");
        assert_eq!(unchanged.header("etag"), etag);
    }
}

#[tokio::test]
async fn replacing_a_picture_changes_its_version_and_its_etag() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    put_image(&app, &cookie, "logo-dark", &file(PNG_HEAD, 100)).await;
    let before = send(&app, None, "GET", BRANDING, None, &[]).await.json();
    let first_etag = send(
        &app,
        None,
        "GET",
        &format!("{BRANDING}/images/logo-dark"),
        None,
        &[],
    )
    .await
    .header("etag")
    .to_owned();
    put_image(&app, &cookie, "logo-dark", &file(PNG_HEAD, 101)).await;
    let after = send(&app, None, "GET", BRANDING, None, &[]).await.json();
    assert_ne!(before["logoDark"]["version"], after["logoDark"]["version"]);
    let second = send(
        &app,
        None,
        "GET",
        &format!("{BRANDING}/images/logo-dark"),
        None,
        &[],
    )
    .await;
    assert_ne!(first_etag, second.header("etag"));
}

#[tokio::test]
async fn a_file_that_breaks_a_rule_is_refused_with_its_sentence_and_nothing_is_stored() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    let svg =
        br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#.to_vec();
    let cases: Vec<(&str, &str, Vec<u8>, &str)> = vec![
        ("an SVG logo", "logo", svg.clone(), LOGO_TYPE),
        ("an SVG dark logo", "logo-dark", svg, LOGO_TYPE),
        (
            "a text file named logo.png",
            "logo",
            b"just some words, not a picture".to_vec(),
            LOGO_TYPE,
        ),
        ("an empty file", "logo", Vec::new(), LOGO_TYPE),
        (
            "a PNG one byte over 512 KB",
            "logo",
            file(PNG_HEAD, 524_289),
            LOGO_SIZE,
        ),
        (
            "a 2 MB PNG, over the route's body limit",
            "logo",
            file(PNG_HEAD, 2 * 1024 * 1024),
            LOGO_SIZE,
        ),
        ("a JPEG icon", "icon", file(JPEG_HEAD, 100), ICON_RULE),
        (
            "a 65 KB PNG icon",
            "icon",
            file(PNG_HEAD, 65 * 1024),
            ICON_RULE,
        ),
    ];
    for (what, kind, bytes, sentence) in cases {
        // The file name and the part type say PNG: the server must not believe them.
        let reply = put_image(&app, &cookie, kind, &bytes).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{what}");
        assert!(
            reply.json().to_string().contains(sentence),
            "{what}: {}",
            reply.json()
        );
        assert_eq!(stored_rows(&app).await, 0, "{what} stored something");
    }
}

/// The edges the rules allow: 512 KB exactly for a logo, 64 KB exactly for an icon.
#[tokio::test]
async fn a_logo_of_512_kb_and_an_icon_of_64_kb_are_accepted() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    assert_eq!(
        put_image(&app, &cookie, "logo", &file(WEBP_HEAD, 524_288))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        put_image(&app, &cookie, "icon", &file(PNG_HEAD, 65_536))
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_kind_outside_the_list_is_a_404_on_the_admin_routes_and_a_400_on_the_public_one() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    let put = put_image(&app, &cookie, "banner", &file(PNG_HEAD, 50)).await;
    assert_eq!(put.status, StatusCode::NOT_FOUND);
    let delete = send(
        &app,
        Some(&cookie),
        "DELETE",
        &format!("{GENERAL}/images/banner"),
        None,
        &[],
    )
    .await;
    assert_eq!(delete.status, StatusCode::NOT_FOUND);
    let public = send(
        &app,
        None,
        "GET",
        &format!("{BRANDING}/images/banner"),
        None,
        &[],
    )
    .await;
    // The public route answers 400, not 404: `routes::route_policy_tests` reads
    // every 404 as "this table entry is not mounted", and it probes `{kind}` with `x`.
    assert_eq!(public.status, StatusCode::BAD_REQUEST);
    assert_eq!(stored_rows(&app).await, 0);
}

#[tokio::test]
async fn a_form_without_a_file_part_is_refused() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    let reply = send(
        &app,
        Some(&cookie),
        "PUT",
        &format!("{GENERAL}/images/logo"),
        Some((
            "multipart/form-data; boundary=BOUNDARY".to_owned(),
            b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"other\"\r\n\r\nx\r\n--BOUNDARY--\r\n".to_vec(),
        )),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(stored_rows(&app).await, 0);
}

#[tokio::test]
async fn an_image_that_is_not_set_is_a_404_on_the_public_route() {
    let app = common::spin_up().await;
    for kind in ["logo", "logo-dark", "icon"] {
        let reply = send(
            &app,
            None,
            "GET",
            &format!("{BRANDING}/images/{kind}"),
            None,
            &[],
        )
        .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{kind}");
    }
}

#[tokio::test]
async fn the_public_answer_needs_no_session_and_holds_only_the_name_and_picture_metadata() {
    let app = common::spin_up().await;
    let fresh = send(&app, None, "GET", BRANDING, None, &[]).await;
    assert_eq!(fresh.status, StatusCode::OK);
    assert_eq!(
        fresh.json(),
        json!({ "workspaceName": null, "logo": null, "logoDark": null, "icon": null })
    );

    let cookie = admin(&app).await;
    assert_eq!(
        put_name(&app, &cookie, "Meridian Data").await.status,
        StatusCode::OK
    );
    let logo = file(PNG_HEAD, 120);
    put_image(&app, &cookie, "logo", &logo).await;
    put_image(&app, &cookie, "icon", &file(PNG_HEAD, 80)).await;

    let reply = send(&app, None, "GET", BRANDING, None, &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["icon", "logo", "logoDark", "workspaceName"]);
    assert_eq!(body["workspaceName"], "Meridian Data");
    assert_eq!(body["logoDark"], Value::Null);
    for kind in ["logo", "icon"] {
        let image = body[kind].as_object().unwrap();
        let mut fields: Vec<&str> = image.keys().map(String::as_str).collect();
        fields.sort_unstable();
        assert_eq!(fields, ["type", "version"], "{kind}");
        assert_eq!(image["type"], "image/png");
        assert!(image["version"].as_str().is_some_and(|v| !v.is_empty()));
    }
    // Nothing of the file, and nothing about who saved it, is in the answer.
    let text = reply.json().to_string();
    assert!(!text.contains("updated"), "{text}");
    assert!(!text.contains("bytes"), "{text}");
}

/// The cache is cleared by every write in the process that made it: a read
/// that filled it before the write must not hide the write.
#[tokio::test]
async fn a_write_is_visible_to_the_next_public_read_at_once() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    let before = send(&app, None, "GET", BRANDING, None, &[]).await.json();
    assert_eq!(before["workspaceName"], Value::Null);
    put_name(&app, &cookie, "Acme").await;
    let after = send(&app, None, "GET", BRANDING, None, &[]).await.json();
    assert_eq!(after["workspaceName"], "Acme");
    send(&app, Some(&cookie), "DELETE", GENERAL, None, &[]).await;
    let reset = send(&app, None, "GET", BRANDING, None, &[]).await.json();
    assert_eq!(reset["workspaceName"], Value::Null);
}

#[tokio::test]
async fn the_admin_answer_has_the_name_each_picture_and_its_size_but_no_bytes() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    let empty = send(&app, Some(&cookie), "GET", GENERAL, None, &[]).await;
    assert_eq!(empty.status, StatusCode::OK);
    let empty = empty.json();
    assert_eq!(empty["workspaceName"], Value::Null);
    assert_eq!(
        empty["logo"],
        json!({ "set": false, "type": null, "bytes": null })
    );

    put_name(&app, &cookie, "  Acme  ").await;
    put_image(&app, &cookie, "logo-dark", &file(JPEG_HEAD, 321)).await;
    let reply = send(&app, Some(&cookie), "GET", GENERAL, None, &[]).await;
    let body = reply.json();
    assert_eq!(body["workspaceName"], "Acme", "the name is trimmed");
    assert_eq!(
        body["logoDark"],
        json!({ "set": true, "type": "image/jpeg", "bytes": 321 })
    );
    assert_eq!(body["logo"]["set"], false);
    assert!(body["updatedAt"].is_string(), "{body}");
    assert!(reply.body.len() < 1024, "no picture inside the answer");
}

#[tokio::test]
async fn a_name_over_60_characters_or_with_a_control_character_is_refused() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    assert_eq!(
        put_name(&app, &cookie, &"a".repeat(61)).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for bad in ["Ac\u{0}me", "Ac\nme", "Ac\u{7f}me"] {
        assert_eq!(
            put_name(&app, &cookie, bad).await.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{bad:?}"
        );
    }
    // 60 characters of two bytes each are fine: the limit counts characters.
    assert_eq!(
        put_name(&app, &cookie, &"é".repeat(60)).await.status,
        StatusCode::OK
    );
    assert_eq!(
        stored_rows(&app).await,
        1,
        "only the accepted name is stored"
    );
    // A body that is not the one field is refused, as every settings body is.
    let extra = send(
        &app,
        Some(&cookie),
        "PUT",
        GENERAL,
        Some((
            "application/json".to_owned(),
            br#"{"workspaceName":"x","logo":"y"}"#.to_vec(),
        )),
        &[],
    )
    .await;
    assert_eq!(extra.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_empty_name_clears_it_and_spaces_alone_do_too() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    put_name(&app, &cookie, "Acme").await;
    assert_eq!(put_name(&app, &cookie, "").await.status, StatusCode::OK);
    let body = send(&app, None, "GET", BRANDING, None, &[]).await.json();
    assert_eq!(body["workspaceName"], Value::Null);
    put_name(&app, &cookie, "Acme").await;
    assert_eq!(put_name(&app, &cookie, "   ").await.status, StatusCode::OK);
    let body = send(&app, None, "GET", BRANDING, None, &[]).await.json();
    assert_eq!(body["workspaceName"], Value::Null);
}

#[tokio::test]
async fn removing_one_picture_leaves_the_others() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    put_image(&app, &cookie, "logo", &file(PNG_HEAD, 50)).await;
    put_image(&app, &cookie, "icon", &file(PNG_HEAD, 60)).await;
    let removed = send(
        &app,
        Some(&cookie),
        "DELETE",
        &format!("{GENERAL}/images/logo"),
        None,
        &[],
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK);
    let body = send(&app, None, "GET", BRANDING, None, &[]).await.json();
    assert_eq!(body["logo"], Value::Null);
    assert!(body["icon"].is_object());
}

#[tokio::test]
async fn reset_clears_the_name_and_every_picture() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    put_name(&app, &cookie, "Acme").await;
    for kind in ["logo", "logo-dark", "icon"] {
        put_image(&app, &cookie, kind, &file(PNG_HEAD, 50)).await;
    }
    let reset = send(&app, Some(&cookie), "DELETE", GENERAL, None, &[]).await;
    assert_eq!(reset.status, StatusCode::OK);
    let body = send(&app, None, "GET", BRANDING, None, &[]).await.json();
    assert_eq!(
        body,
        json!({ "workspaceName": null, "logo": null, "logoDark": null, "icon": null })
    );
    assert_eq!(stored_rows(&app).await, 0);
}

#[tokio::test]
async fn a_caller_without_settings_write_is_refused_and_nothing_is_written() {
    let app = common::spin_up().await;
    let reader = create_zero_permission_principal(&app.pool).await;
    let cookie = session_cookie_for_user(&app.pool, reader).await;
    assert_eq!(
        send(&app, Some(&cookie), "GET", GENERAL, None, &[])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        put_name(&app, &cookie, "Acme").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        put_image(&app, &cookie, "logo", &file(PNG_HEAD, 50))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(stored_rows(&app).await, 0);
    // The gate records the refusal itself (outcome `refused`); no save was
    // executed, so no row says one was.
    for action in ["settings.general.update", "settings.general.image_upload"] {
        for row in audit_rows(&app.pool, action).await {
            assert_eq!(row["outcome"], "refused", "{action}: {row}");
        }
    }
}

/// Every write leaves one audit row with its action; the image routes name the
/// kind, the type and the size and never the file.
#[tokio::test]
async fn every_write_leaves_one_audit_row_and_never_the_picture() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;

    put_name(&app, &cookie, "Acme").await;
    let updates = audit_rows(&app.pool, "settings.general.update").await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["args"], json!({ "fields": ["workspaceName"] }));

    let marker = b"SECRET-PICTURE-MARKER";
    let mut bytes = file(PNG_HEAD, 40);
    bytes.extend_from_slice(marker);
    put_image(&app, &cookie, "logo-dark", &bytes).await;
    let uploads = audit_rows(&app.pool, "settings.general.image_upload").await;
    assert_eq!(uploads.len(), 1);
    assert_eq!(
        uploads[0]["args"],
        json!({ "kind": "logo-dark", "type": "image/png", "bytes": bytes.len() })
    );

    send(
        &app,
        Some(&cookie),
        "DELETE",
        &format!("{GENERAL}/images/logo-dark"),
        None,
        &[],
    )
    .await;
    let removals = audit_rows(&app.pool, "settings.general.image_remove").await;
    assert_eq!(removals.len(), 1);
    assert_eq!(
        removals[0]["args"],
        json!({ "kind": "logo-dark", "type": "image/png", "bytes": bytes.len() })
    );

    send(&app, Some(&cookie), "DELETE", GENERAL, None, &[]).await;
    assert_eq!(
        audit_rows(&app.pool, "settings.general.reset").await.len(),
        1
    );

    // No row of the four carries the file, whatever the column.
    for action in [
        "settings.general.update",
        "settings.general.image_upload",
        "settings.general.image_remove",
        "settings.general.reset",
    ] {
        for row in audit_rows(&app.pool, action).await {
            let text = row.to_string();
            assert!(!text.contains("SECRET-PICTURE-MARKER"), "{action}: {text}");
            assert!(!text.contains("U0VDUkVU"), "{action} holds base64 of it");
        }
    }
}

/// A refused write is not a write: no audit row either.
#[tokio::test]
async fn a_refused_upload_leaves_no_audit_row() {
    let app = common::spin_up().await;
    let cookie = admin(&app).await;
    put_image(&app, &cookie, "icon", &file(JPEG_HEAD, 50)).await;
    assert_eq!(
        audit_rows(&app.pool, "settings.general.image_upload").await,
        Vec::<Value>::new(),
        "settings.general.image_upload leaves no row for a refused upload"
    );
}
