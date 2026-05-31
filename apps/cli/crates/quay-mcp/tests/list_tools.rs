//! Protocol test: spin up the server on an in-memory duplex, list tools,
//! assert the search tool is present with the read-only annotation and an
//! input schema. No network — does not call any tool.

use quay_mcp::test_support::test_server;
use rmcp::ServiceExt;

#[tokio::test]
async fn lists_search_tool_with_readonly_annotation() {
    let (client_io, server_io) = tokio::io::duplex(4096);

    let server = test_server();
    let server_handle = tokio::spawn(async move {
        let running = server.serve(server_io).await.unwrap();
        running.waiting().await.unwrap();
    });

    let client = ().serve(client_io).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();

    let search = tools
        .iter()
        .find(|t| t.name == "quay_search")
        .expect("quay_search present");
    assert_eq!(
        search.annotations.as_ref().and_then(|a| a.read_only_hint),
        Some(true)
    );
    assert_eq!(
        search.annotations.as_ref().and_then(|a| a.open_world_hint),
        Some(true)
    );
    assert!(search.input_schema.get("properties").is_some());

    client.cancel().await.unwrap();
    server_handle.abort();
    let _ = server_handle.await;
}

#[tokio::test]
async fn lists_all_read_tools_with_correct_annotations() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let server = test_server();
    let h = tokio::spawn(async move {
        server.serve(server_io).await.unwrap().waiting().await.unwrap();
    });
    let client = ().serve(client_io).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();

    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in [
        "quay_search",
        "quay_info",
        "quay_list",
        "quay_outdated",
        "quay_scan",
        "quay_validate",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
    let read_tools = [
        "quay_search",
        "quay_info",
        "quay_list",
        "quay_outdated",
        "quay_scan",
        "quay_validate",
    ];
    for t in &tools {
        if read_tools.contains(&t.name.as_ref()) {
            let ro = t.annotations.as_ref().and_then(|a| a.read_only_hint);
            assert_eq!(ro, Some(true), "{} should be read_only", t.name);
        }
    }
    for t in &tools {
        if ["quay_info", "quay_outdated"].contains(&t.name.as_ref()) {
            let ow = t.annotations.as_ref().and_then(|a| a.open_world_hint);
            assert_eq!(ow, Some(true), "{} should be open_world", t.name);
        }
    }

    client.cancel().await.unwrap();
    h.abort();
    let _ = h.await;
}

#[tokio::test]
async fn write_tools_are_not_read_only_and_remove_is_destructive() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let server = test_server();
    let h = tokio::spawn(async move {
        server.serve(server_io).await.unwrap().waiting().await.unwrap();
    });
    let client = ().serve(client_io).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let by = |n: &str| tools.iter().find(|t| t.name == n).unwrap().clone();

    for n in ["quay_add", "quay_link", "quay_update", "quay_remove"] {
        let a = by(n).annotations.unwrap();
        assert_ne!(a.read_only_hint, Some(true), "{n} must not be read_only");
    }
    assert_eq!(
        by("quay_remove").annotations.unwrap().destructive_hint,
        Some(true)
    );

    client.cancel().await.unwrap();
    h.abort();
    let _ = h.await;
}

#[tokio::test]
async fn outward_tools_advertise_open_world() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let server = test_server();
    let h = tokio::spawn(async move {
        server.serve(server_io).await.unwrap().waiting().await.unwrap();
    });
    let client = ().serve(client_io).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let by = |n: &str| tools.iter().find(|t| t.name == n).unwrap().clone();

    for n in ["quay_push", "quay_remote"] {
        let a = by(n).annotations.unwrap();
        assert_eq!(a.open_world_hint, Some(true), "{n} must be open_world");
        assert_ne!(a.read_only_hint, Some(true), "{n} must not be read_only");
    }

    client.cancel().await.unwrap();
    h.abort();
    let _ = h.await;
}
