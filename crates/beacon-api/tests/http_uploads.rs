//! Tests for the multipart upload endpoint + attachment claim flow.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;

fn multipart_body(parts: Vec<(&str, &str, Option<&str>, Vec<u8>)>) -> (String, Vec<u8>) {
    // boundary is fixed for tests; production uses random.
    let boundary = "----PaschalTestBoundary7d8e9f0a1b2c";
    let mut body = Vec::new();
    for (name, filename, content_type, data) in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        let cd = if filename.is_empty() {
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n")
        } else {
            format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n"
            )
        };
        body.extend_from_slice(cd.as_bytes());
        if let Some(ct) = content_type {
            body.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[tokio::test]
async fn upload_text_and_pdf_attachments() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "upload@example.org").await;

    let (_, vault) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({"name": "Upload", "cooling_off_seconds": 1}),
            Some(&token),
        ),
    )
    .await;
    let vid = vault["id"].as_str().unwrap();

    let (content_type, body) = multipart_body(vec![
        ("title", "", None, b"With attachments".to_vec()),
        ("recipient_email", "", None, b"r@example.org".to_vec()),
        ("body", "", None, b"Letter body here.".to_vec()),
        (
            "file",
            "notes.txt",
            Some("text/plain"),
            b"hello\r\nworld\r\n".to_vec(),
        ),
        (
            "file",
            "scan.pdf",
            Some("application/pdf"),
            b"%PDF-1.7\n%test\n".to_vec(),
        ),
    ]);

    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/vaults/{vid}/letters/multipart"))
        .header("content-type", content_type)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let (status, resp) = common::send(&app.router, req).await;
    assert_eq!(status, StatusCode::OK, "got: {resp:?}");

    let attachments = resp["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 2);

    // First attachment: text normalized to LF, MIME preserved
    let txt = &attachments[0];
    assert_eq!(txt["original_filename"], "notes.txt");
    assert_eq!(txt["original_mime"], "text/plain");
    assert_eq!(txt["transformed_mime"], "text/plain");
    // Original was 14 bytes (hello\r\nworld\r\n), transformed should be 12.
    assert_eq!(txt["original_size"], 14);
    assert_eq!(txt["transformed_size"], 12);
    let notes = txt["transformer_notes"]["notes"].as_array().unwrap();
    assert!(notes.iter().any(|n| n.as_str().unwrap().contains("LF")));

    // Second attachment: pdf header validated
    let pdf = &attachments[1];
    assert_eq!(pdf["original_filename"], "scan.pdf");
    assert_eq!(pdf["transformed_mime"], "application/pdf");

    // SHA-256 hex must be 64 chars
    assert_eq!(txt["sha256_hex"].as_str().unwrap().len(), 64);
}

#[tokio::test]
async fn upload_rejects_invalid_pdf() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "badpdf@example.org").await;
    let (_, vault) = common::send(
        &app.router,
        common::req_post("/v1/vaults", json!({"name": "V"}), Some(&token)),
    )
    .await;
    let vid = vault["id"].as_str().unwrap();

    let (content_type, body) = multipart_body(vec![
        ("title", "", None, b"Test".to_vec()),
        ("recipient_email", "", None, b"r@example.org".to_vec()),
        ("body", "", None, b"x".to_vec()),
        (
            "file",
            "fake.pdf",
            Some("application/pdf"),
            b"not a pdf".to_vec(),
        ),
    ]);

    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/vaults/{vid}/letters/multipart"))
        .header("content-type", content_type)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let (status, body) = common::send(&app.router, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["detail"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("pdf"));
}

#[tokio::test]
async fn upload_rejects_missing_title() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "notitle@example.org").await;
    let (_, vault) = common::send(
        &app.router,
        common::req_post("/v1/vaults", json!({"name": "V"}), Some(&token)),
    )
    .await;
    let vid = vault["id"].as_str().unwrap();

    let (content_type, body) = multipart_body(vec![
        ("recipient_email", "", None, b"r@example.org".to_vec()),
        ("body", "", None, b"x".to_vec()),
    ]);

    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/vaults/{vid}/letters/multipart"))
        .header("content-type", content_type)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let (status, _) = common::send(&app.router, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upload_rejects_body_and_files_both_missing() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "neither@example.org").await;
    let (_, vault) = common::send(
        &app.router,
        common::req_post("/v1/vaults", json!({"name": "V"}), Some(&token)),
    )
    .await;
    let vid = vault["id"].as_str().unwrap();

    let (content_type, body) = multipart_body(vec![
        ("title", "", None, b"T".to_vec()),
        ("recipient_email", "", None, b"r@example.org".to_vec()),
    ]);

    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/vaults/{vid}/letters/multipart"))
        .header("content-type", content_type)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let (status, _) = common::send(&app.router, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn end_to_end_release_with_attachment() {
    let app = common::setup().await;
    let (_, token) = common::signup_and_get_token(&app.router, "e2e-att@example.org").await;

    let (_, vault) = common::send(
        &app.router,
        common::req_post(
            "/v1/vaults",
            json!({"name": "V", "cooling_off_seconds": 1}),
            Some(&token),
        ),
    )
    .await;
    let vid = vault["id"].as_str().unwrap();

    // Upload a Letter with an attachment.
    let (content_type, body) = multipart_body(vec![
        ("title", "", None, b"With attachment".to_vec()),
        ("recipient_email", "", None, b"r@example.org".to_vec()),
        ("body", "", None, b"Body text".to_vec()),
        (
            "file",
            "instructions.txt",
            Some("text/plain"),
            b"step 1\r\nstep 2\r\n".to_vec(),
        ),
    ]);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/vaults/{vid}/letters/multipart"))
        .header("content-type", content_type)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let (status, _resp) = common::send(&app.router, req).await;
    assert_eq!(status, StatusCode::OK);

    // Force-release.
    common::send(
        &app.router,
        common::req_post(
            &format!("/v1/vaults/{vid}/force-release"),
            json!({}),
            Some(&token),
        ),
    )
    .await;

    // Wait for cooling-off + release.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Fetch the attachment claim token from the DB directly (the email body
    // would carry it in production).
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT a.id, a.original_filename, a.transformed_mime
           FROM attachment a
           JOIN letter l ON l.id = a.letter_id
          WHERE l.vault_id = $1
          LIMIT 1",
    )
    .bind(uuid::Uuid::parse_str(vid).unwrap())
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let attachment_id: uuid::Uuid = row.get("id");
    let filename: String = row.get("original_filename");
    let mime: String = row.get("transformed_mime");

    // Issue a fresh claim token directly (simulating consuming the email link).
    let att_token = crypto_stub::random_token();
    sqlx::query(
        "INSERT INTO release_attachment_claim
            (token_hash, release_event_id, attachment_id, recipient_email, expires_at)
         SELECT $1, re.id, $2, 'r@example.org', now() + interval '1 day'
           FROM release_event re
           JOIN letter l ON l.vault_id = re.vault_id
          WHERE l.vault_id = $3 AND re.released_at IS NOT NULL
          ORDER BY re.triggered_at DESC LIMIT 1",
    )
    .bind(&crypto_stub::hash_token(&att_token)[..])
    .bind(attachment_id)
    .bind(uuid::Uuid::parse_str(vid).unwrap())
    .execute(&app.pool)
    .await
    .unwrap();

    // GET the attachment claim.
    let resp = tower::ServiceExt::oneshot(
        app.router.clone(),
        Request::builder()
            .method("GET")
            .uri(format!("/v1/releases/claim/attachment?token={att_token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert_eq!(ct, mime);
    let cd = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(cd.contains(&filename), "got disposition: {cd}");
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    // The plaintext should be the LF-normalised version: "step 1\nstep 2\n".
    assert_eq!(&body[..], b"step 1\nstep 2\n");

    // Second claim must fail.
    let resp = tower::ServiceExt::oneshot(
        app.router.clone(),
        Request::builder()
            .method("GET")
            .uri(format!("/v1/releases/claim/attachment?token={att_token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
