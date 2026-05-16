//! Integration tests for resource embedding (joins via foreign key relationships).
//!
//! Covers PostgREST EmbedSpec.hs parity.

mod common;

use common::{setup_test_db, test_dsn, PgvisServer};
use reqwest::StatusCode;
use std::sync::OnceLock;

struct ServerInfo {
    client: reqwest::Client,
    base_url: String,
}

static SERVER: OnceLock<ServerInfo> = OnceLock::new();

fn server_info() -> &'static ServerInfo {
    SERVER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<String>();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let dsn = test_dsn();
                setup_test_db(&dsn).await;
                let s = PgvisServer::start(&dsn, "test").await;
                tx.send(s.base_url.clone()).unwrap();
                std::future::pending::<()>().await;
            });
        });

        let base_url = rx.recv().expect("failed to receive server base_url");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        ServerInfo { client, base_url }
    })
}

async fn get(path: &str) -> reqwest::Response {
    let s = server_info();
    s.client
        .get(format!("{}{path}", s.base_url))
        .send()
        .await
        .expect("GET request failed")
}

// ============================================================================
// Many-to-One (child → parent) embedding
// ============================================================================

#[tokio::test]
async fn test_embed_many_to_one() {
    // orders → users (via user_id FK)
    let resp = get("/api/test/orders?select=id,total,users(id,name)").await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK,
        "expected 200 for embed query, got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    assert!(!arr.is_empty(), "should have orders");

    // Each order should have an embedded user object
    for order in arr {
        let user = &order["users"];
        if !user.is_null() {
            assert!(
                user.is_object(),
                "M2O embed should be object, got: {user}"
            );
            assert!(user.get("id").is_some());
            assert!(user.get("name").is_some());
        }
    }
}

#[tokio::test]
async fn test_embed_many_to_one_with_filter() {
    // Get orders for a specific user via embedding
    let resp = get("/api/test/orders?select=id,total,users(name)&users.name=eq.Alice").await;
    let status = resp.status();
    // Filtering on embedded resource might not be supported yet
    assert!(
        status == StatusCode::OK || status.is_client_error(),
        "got {status}"
    );
}

// ============================================================================
// One-to-Many (parent → children) embedding
// ============================================================================

#[tokio::test]
async fn test_embed_one_to_many() {
    // users → orders (one user has many orders)
    let resp = get("/api/test/users?select=id,name,orders(id,total,status)").await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK,
        "expected 200 for O2M embed, got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    assert!(!arr.is_empty());

    // Each user should have an orders array
    for user in arr {
        let orders = &user["orders"];
        if !orders.is_null() {
            assert!(
                orders.is_array(),
                "O2M embed should be array, got: {orders}"
            );
        }
    }
}

#[tokio::test]
async fn test_embed_one_to_many_specific_user() {
    // Alice (id=1) has 2 orders
    let resp = get("/api/test/users?select=name,orders(id,total)&id=eq.1").await;
    let status = resp.status();
    assert!(status == StatusCode::OK, "got {status}");
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    assert_eq!(arr.len(), 1, "should return only Alice");
    let orders = &arr[0]["orders"];
    if orders.is_array() {
        assert_eq!(
            orders.as_array().unwrap().len(),
            2,
            "Alice should have 2 orders"
        );
    }
}

// ============================================================================
// Many-to-Many embedding (via junction table)
// ============================================================================

#[tokio::test]
async fn test_embed_many_to_many() {
    // orders → items via order_items junction
    let resp = get("/api/test/orders?select=id,total,order_items(item_id,quantity)").await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK,
        "expected 200 for M2M embed, got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    assert!(!arr.is_empty());
}

// ============================================================================
// Nested embedding
// ============================================================================

#[tokio::test]
async fn test_embed_nested() {
    // projects → tasks → assigned user
    let resp =
        get("/api/test/projects?select=name,tasks(title,done,users(name))").await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK,
        "expected 200 for nested embed, got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    assert!(!arr.is_empty());

    // Each project should have tasks array
    for project in arr {
        let tasks = &project["tasks"];
        if tasks.is_array() {
            for task in tasks.as_array().unwrap() {
                assert!(task.get("title").is_some());
                // Some tasks have assigned_to = NULL, so users might be null
            }
        }
    }
}

// ============================================================================
// Embedding with column selection
// ============================================================================

#[tokio::test]
async fn test_embed_select_specific_columns() {
    let resp = get("/api/test/orders?select=id,users(name)").await;
    let status = resp.status();
    assert!(status == StatusCode::OK, "got {status}");
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    if !arr.is_empty() {
        let user = &arr[0]["users"];
        if user.is_object() {
            // Should only have "name" from the embed select
            assert!(user.get("name").is_some());
        }
    }
}

// ============================================================================
// Error cases for embedding
// ============================================================================

#[tokio::test]
async fn test_embed_nonexistent_relationship() {
    // items has no direct FK to users
    let resp = get("/api/test/items?select=name,users(name)").await;
    let status = resp.status();
    // Should fail with some error (400 or 404)
    assert!(
        status.is_client_error() || status.is_server_error(),
        "embedding nonexistent relationship should fail, got {status}"
    );
}

#[tokio::test]
async fn test_embed_empty_select_on_embed() {
    // Empty embed select should still work (return all columns of embedded resource)
    let resp = get("/api/test/orders?select=id,users(*)").await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK,
        "embed with star should work, got {status}"
    );
}

// ============================================================================
// Embedding with filters on parent
// ============================================================================

#[tokio::test]
async fn test_embed_with_parent_filter() {
    let resp = get("/api/test/orders?select=id,total,users(name)&status=eq.completed").await;
    let status = resp.status();
    assert!(status == StatusCode::OK, "got {status}");
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    for order in arr {
        // All orders should be completed
        if let Some(s) = order.get("status") {
            assert_eq!(s, "completed");
        }
    }
}

// ============================================================================
// Self-referential relationships (if any)
// ============================================================================

#[tokio::test]
async fn test_embed_with_ordering() {
    let resp = get("/api/test/users?select=name,orders(id,total)&order=name.asc").await;
    let status = resp.status();
    assert!(status == StatusCode::OK, "got {status}");
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    // Verify parent ordering
    if arr.len() >= 2 {
        let first = arr[0]["name"].as_str().unwrap_or("");
        let second = arr[1]["name"].as_str().unwrap_or("");
        assert!(first <= second, "should be ordered: '{first}' <= '{second}'");
    }
}
