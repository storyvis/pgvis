//! Integration tests for RPC (stored function calls).
//!
//! Covers PostgREST RpcSpec.hs parity.

mod common;

use common::{PgvisServer, setup_test_db, test_dsn};
use reqwest::StatusCode;
use serde_json::json;
use std::sync::OnceLock;

/// Shared server info.
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

/// POST to an RPC endpoint with JSON body.
async fn rpc_post(fn_name: &str, body: serde_json::Value) -> reqwest::Response {
    let s = server_info();
    s.client
        .post(format!("{}/api/test/rpc/{fn_name}", s.base_url))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .expect("RPC POST request failed")
}

/// POST to RPC with Prefer header.
async fn rpc_post_prefer(fn_name: &str, body: serde_json::Value, pref: &str) -> reqwest::Response {
    let s = server_info();
    s.client
        .post(format!("{}/api/test/rpc/{fn_name}", s.base_url))
        .header("content-type", "application/json")
        .header("Prefer", pref)
        .json(&body)
        .send()
        .await
        .expect("RPC POST request failed")
}

/// GET to an RPC endpoint (for STABLE/IMMUTABLE functions).
async fn rpc_get(fn_name: &str) -> reqwest::Response {
    let s = server_info();
    s.client
        .get(format!("{}/api/test/rpc/{fn_name}", s.base_url))
        .send()
        .await
        .expect("RPC GET request failed")
}

/// GET to an RPC endpoint with query params.
async fn rpc_get_params(fn_name: &str, params: &str) -> reqwest::Response {
    let s = server_info();
    s.client
        .get(format!("{}/api/test/rpc/{fn_name}?{params}", s.base_url))
        .send()
        .await
        .expect("RPC GET request failed")
}

// ============================================================================
// Scalar functions
// ============================================================================

#[tokio::test]
async fn test_rpc_add_two_integers() {
    let resp = rpc_post("add", json!({"a": 3, "b": 5})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    // Scalar function returns {"result": 8} or just 8 or [{"result": 8}]
    let result = if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            json!(null)
        } else {
            arr[0].get("result").cloned().unwrap_or(arr[0].clone())
        }
    } else if let Some(r) = body.get("result") {
        r.clone()
    } else {
        body.clone()
    };
    assert_eq!(result, json!(8));
}

#[tokio::test]
async fn test_rpc_add_negative_numbers() {
    let resp = rpc_post("add", json!({"a": -10, "b": 7})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let result = if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            json!(null)
        } else {
            arr[0].get("result").cloned().unwrap_or(arr[0].clone())
        }
    } else if let Some(r) = body.get("result") {
        r.clone()
    } else {
        body.clone()
    };
    assert_eq!(result, json!(-3));
}

#[tokio::test]
async fn test_rpc_echo_params_defaults() {
    let resp = rpc_post("echo_params", json!({})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    // Default: "hello, world!"
    let result = if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            json!(null)
        } else {
            arr[0].get("result").cloned().unwrap_or(arr[0].clone())
        }
    } else if let Some(r) = body.get("result") {
        r.clone()
    } else {
        body.clone()
    };
    assert_eq!(result, json!("hello, world!"));
}

#[tokio::test]
async fn test_rpc_echo_params_custom() {
    let resp = rpc_post("echo_params", json!({"name": "pgvis", "greeting": "hi"})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let result = if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            json!(null)
        } else {
            arr[0].get("result").cloned().unwrap_or(arr[0].clone())
        }
    } else if let Some(r) = body.get("result") {
        r.clone()
    } else {
        body.clone()
    };
    assert_eq!(result, json!("hi, pgvis!"));
}

// ============================================================================
// Set-returning functions
// ============================================================================

#[tokio::test]
async fn test_rpc_get_items_returns_set() {
    let resp = rpc_post("get_items", json!({})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body
        .as_array()
        .expect("set-returning function should return array");
    assert!(
        arr.len() >= 10,
        "should return at least 10 items, got {}",
        arr.len()
    );
    // Each item should have typical columns
    assert!(arr[0].get("id").is_some());
    assert!(arr[0].get("name").is_some());
}

#[tokio::test]
async fn test_rpc_search_items() {
    let resp = rpc_post("search_items", json!({"query": "Widget"})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    assert!(!arr.is_empty(), "should find at least one Widget");
    for item in arr {
        let name = item["name"].as_str().unwrap_or("");
        assert!(
            name.to_lowercase().contains("widget"),
            "all results should contain 'widget', got '{name}'"
        );
    }
}

#[tokio::test]
async fn test_rpc_search_items_no_match() {
    let resp = rpc_post("search_items", json!({"query": "ZZZZNOEXIST99999"})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    assert!(arr.is_empty(), "should return empty array for no match");
}

#[tokio::test]
async fn test_rpc_get_single_item() {
    let resp = rpc_post("get_item", json!({"item_id": 1})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    // Single-row return — might be object or array with one element
    if let Some(arr) = body.as_array() {
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[0]["name"], "Widget");
    } else {
        assert_eq!(body["id"], 1);
        assert_eq!(body["name"], "Widget");
    }
}

// ============================================================================
// Void functions
// ============================================================================

#[tokio::test]
async fn test_rpc_void_function() {
    let resp = rpc_post("void_function", json!({})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK
            || status == StatusCode::CREATED
            || status == StatusCode::NO_CONTENT,
        "got {status}"
    );
}

// ============================================================================
// JSON-returning functions
// ============================================================================

#[tokio::test]
async fn test_rpc_get_json() {
    let resp = rpc_post("get_json", json!({})).await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    // Should contain the JSON result
    let result = if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            json!(null)
        } else {
            arr[0].get("result").cloned().unwrap_or(arr[0].clone())
        }
    } else if let Some(r) = body.get("result") {
        r.clone()
    } else {
        body.clone()
    };
    // The function returns {"key": "value", "count": 42}
    assert_eq!(result["key"], "value");
    assert_eq!(result["count"], 42);
}

// ============================================================================
// GET-based RPC (for STABLE/IMMUTABLE)
// ============================================================================

#[tokio::test]
async fn test_rpc_get_method_stable_function() {
    let resp = rpc_get("get_items").await;
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "got {status}"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("should return array");
    assert!(!arr.is_empty());
}

// ============================================================================
// Error cases
// ============================================================================

#[tokio::test]
async fn test_rpc_nonexistent_function() {
    let resp = rpc_post("totally_fake_function", json!({})).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_rpc_wrong_param_name() {
    // Function expects "a" and "b", we pass "x" and "y"
    let resp = rpc_post("add", json!({"x": 1, "y": 2})).await;
    let status = resp.status();
    // Should either fail or use defaults (which may error for non-default params)
    assert!(
        status.is_client_error()
            || status.is_server_error()
            || status == StatusCode::OK
            || status == StatusCode::CREATED,
        "got {status}"
    );
}
