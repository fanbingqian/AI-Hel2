use crate::services::knowledge_service::KnowledgeService;
use std::sync::Arc;

/// Start the Nexus HTTP API server on 127.0.0.1, binding to the first available port
/// starting from `base_port`. Writes the actual port to `{hermes_home}/nexus_port`.
/// Runs in a background thread until the process exits.
pub fn start(knowledge: Arc<KnowledgeService>, hermes_home: &std::path::Path) {
    let base_port = 18643u16;
    let max_attempts = 10u16;

    let mut port = base_port;
    let server = loop {
        match tiny_http::Server::http(format!("127.0.0.1:{port}")) {
            Ok(s) => break s,
            Err(_) if port - base_port < max_attempts => {
                log::warn!("[Nexus API] Port {port} busy, trying next...");
                port += 1;
            }
            Err(e) => {
                log::error!("[Nexus API] Failed to bind any port: {e}");
                return;
            }
        }
    };

    let port_path = hermes_home.join("nexus_port");
    if let Err(e) = std::fs::write(&port_path, port.to_string()) {
        log::warn!("[Nexus API] Failed to write nexus_port file: {e}");
    } else {
        log::info!("[Nexus API] Port {port} written to {}", port_path.display());
    }

    std::thread::spawn(move || {
        log::info!("[Nexus API] Listening on 127.0.0.1:{port}");

        for mut req in server.incoming_requests() {
            let url = req.url().to_string();
            let method = req.method().as_str().to_string();

            // POST /mcp — MCP JSON-RPC endpoint
            if method == "POST" && url == "/mcp" {
                if let Some(response) = crate::services::nexus_mcp::handle_mcp_request(&mut req) {
                    let _ = req.respond(response);
                } else {
                    let _ = req.respond(
                        tiny_http::Response::from_string("Internal error")
                            .with_status_code(500),
                    );
                }
                continue;
            }

            let is_maintenance_post = method == "POST" && url.starts_with("/nexus/maintain/");
            if method != "GET" && !is_maintenance_post {
                let _ = req.respond(
                    tiny_http::Response::from_string("Method not allowed")
                        .with_status_code(405),
                );
                continue;
            }

            let response = handle_request(&knowledge, &url);
            let _ = req.respond(response);
        }

        log::info!("[Nexus API] Server stopped");
    });
}

fn handle_request(
    knowledge: &Arc<KnowledgeService>,
    url: &str,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let path = url.split('?').next().unwrap_or(url);

    // GET /nexus/map
    if path == "/nexus/map" {
        return match knowledge.build_knowledge_map() {
            Ok(json) => json_response(&json.to_string()),
            Err(e) => error_response(&e),
        };
    }

    // GET /nexus/search?q=&namespace=&limit=
    if path == "/nexus/search" {
        let q = get_query_param(url, "q").unwrap_or_default();
        if q.is_empty() {
            return json_response(r#"{"entities":[]}"#);
        }
        let namespace = get_query_param(url, "namespace");
        let limit: Option<u32> = get_query_param(url, "limit")
            .and_then(|l| l.parse().ok());

        return match knowledge.search_entities_local(&q, namespace.as_deref(), limit) {
            Ok(results) => {
                let body = serde_json::json!({"entities": results}).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // GET /nexus/entity/{id}
    if let Some(id) = path.strip_prefix("/nexus/entity/") {
        let id = id.trim_matches('/');
        if id.is_empty() {
            return tiny_http::Response::from_string("Missing entity id")
                .with_status_code(400);
        }
        return match knowledge.get_entity_detail_local(id) {
            Ok(detail) => {
                let body = serde_json::json!(detail).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // GET /nexus/paths?from=&to=&max_hops=
    if path == "/nexus/paths" {
        let from = get_query_param(url, "from").unwrap_or_default();
        let to = get_query_param(url, "to").unwrap_or_default();
        if from.is_empty() || to.is_empty() {
            return tiny_http::Response::from_string("Missing from/to parameters")
                .with_status_code(400);
        }
        let max_hops: u32 = get_query_param(url, "max_hops")
            .and_then(|h| h.parse().ok())
            .unwrap_or(4);

        let from_id = resolve_entity_id(knowledge, &from);
        let to_id = resolve_entity_id(knowledge, &to);

        return match knowledge.find_paths_local(&from_id, &to_id, max_hops) {
            Ok(result) => {
                let body = serde_json::json!(result).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // ── Maintenance endpoints ──

    // GET /nexus/maintain/health
    if path == "/nexus/maintain/health" {
        let body = serde_json::json!({"status": "ok", "service": "nexus"}).to_string();
        return json_response(&body);
    }

    // GET /nexus/maintain/status
    if path == "/nexus/maintain/status" {
        return match knowledge.nexus_get_maintenance_status() {
            Ok(status) => {
                let body = serde_json::json!(status).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/dedup
    if path == "/nexus/maintain/dedup" {
        return match knowledge.nexus_maintain_dedup() {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/quality
    if path == "/nexus/maintain/quality" {
        return match knowledge.nexus_maintain_quality() {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/cleanup
    if path == "/nexus/maintain/cleanup" {
        return match knowledge.nexus_maintain_cleanup() {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/fix-migrated
    if path == "/nexus/maintain/fix-migrated" {
        return match knowledge.nexus_maintain_fix_migrated() {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/classify
    if path == "/nexus/maintain/classify" {
        return match knowledge.nexus_maintain_classify(false) {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/pagerank
    if path == "/nexus/maintain/pagerank" {
        return match knowledge.nexus_run_pagerank() {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/community
    if path == "/nexus/maintain/community" {
        return match knowledge.nexus_run_community() {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/causal
    if path == "/nexus/maintain/causal" {
        let entity_id = get_query_param(url, "entity_id").unwrap_or_default();
        if entity_id.is_empty() {
            return tiny_http::Response::from_string("Missing entity_id parameter")
                .with_status_code(400);
        }
        let resolved = resolve_entity_id(knowledge, &entity_id);
        return match knowledge.nexus_discover_causal(&resolved) {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/transitive
    if path == "/nexus/maintain/transitive" {
        return match knowledge.nexus_run_transitive() {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/conflicts
    if path == "/nexus/maintain/conflicts" {
        return match knowledge.nexus_scan_conflicts() {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/evolution
    if path == "/nexus/maintain/evolution" {
        let entity_id = get_query_param(url, "entity_id").unwrap_or_default();
        if entity_id.is_empty() {
            return tiny_http::Response::from_string("Missing entity_id parameter")
                .with_status_code(400);
        }
        let resolved = resolve_entity_id(knowledge, &entity_id);
        return match knowledge.nexus_get_evolution(&resolved) {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // POST /nexus/maintain/verify
    if path == "/nexus/maintain/verify" {
        return match knowledge.nexus_verify_synthesis() {
            Ok(report) => {
                let body = serde_json::json!(report).to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    // GET /nexus/neighbors/{id}?hops=
    if let Some(id) = path.strip_prefix("/nexus/neighbors/") {
        let id = id.trim_matches('/');
        if id.is_empty() {
            return tiny_http::Response::from_string("Missing entity id")
                .with_status_code(400);
        }
        let hops: u32 = get_query_param(url, "hops")
            .and_then(|h| h.parse().ok())
            .unwrap_or(2)
            .min(4);

        let entity_id = resolve_entity_id(knowledge, id);

        return match find_neighbors_sync(knowledge, &entity_id, hops) {
            Ok(result) => {
                let body = result.to_string();
                json_response(&body)
            }
            Err(e) => error_response(&e),
        };
    }

    tiny_http::Response::from_string("Not found").with_status_code(404)
}

fn json_response(body: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(body.to_string())
        .with_header("Content-Type: application/json; charset=utf-8".parse::<tiny_http::Header>().unwrap())
}

fn error_response(msg: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::json!({"error": msg}).to_string();
    tiny_http::Response::from_string(body)
        .with_header("Content-Type: application/json; charset=utf-8".parse::<tiny_http::Header>().unwrap())
        .with_status_code(500)
}

/// Simple URL percent-decode. Only handles the common cases used in query strings.
fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            result.push('%');
            result.push_str(&hex);
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

fn get_query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for part in query.split('&') {
        let mut kv = part.splitn(2, '=');
        let k = kv.next()?;
        let v = kv.next().unwrap_or("");
        if k == key {
            return Some(url_decode(v));
        }
    }
    None
}

/// Resolve an entity identifier (name or UUID) to an entity ID.
/// UUIDs are matched exactly; names are resolved via FTS5 fuzzy match.
fn resolve_entity_id(knowledge: &Arc<KnowledgeService>, input: &str) -> String {
    if uuid::Uuid::parse_str(input).is_ok() {
        return input.to_string();
    }

    match knowledge.search_entities_local(input, None, Some(1)) {
        Ok(results) if !results.is_empty() => results[0].id.clone(),
        _ => input.to_string(),
    }
}

/// Synchronous BFS neighbor expansion for the HTTP endpoint.
fn find_neighbors_sync(
    knowledge: &Arc<KnowledgeService>,
    entity_id: &str,
    hops: u32,
) -> Result<serde_json::Value, String> {
    let detail = knowledge.get_entity_detail_local(entity_id)?;

    let entity_json = serde_json::json!({
        "id": detail.entity.id,
        "name": detail.entity.name,
        "entity_type": detail.entity.entity_type,
        "description": detail.entity.description,
        "confidence": detail.entity.confidence,
    });

    // Merge inbound-relations and outbound-relations
    let mut neighbors: Vec<serde_json::Value> = Vec::new();

    for r in &detail.inbound_relations {
        neighbors.push(serde_json::json!({
            "relation_id": r.id,
            "source_id": r.from_id,
            "source_name": find_entity_name(knowledge, &r.from_id),
            "target_id": r.to_id,
            "target_name": detail.entity.name,
            "relation_type": r.relation_type,
            "label": r.label,
            "weight": r.weight,
            "direction": "in",
        }));
    }
    for r in &detail.outbound_relations {
        neighbors.push(serde_json::json!({
            "relation_id": r.id,
            "source_id": r.from_id,
            "source_name": detail.entity.name,
            "target_id": r.to_id,
            "target_name": find_entity_name(knowledge, &r.to_id),
            "relation_type": r.relation_type,
            "label": r.label,
            "weight": r.weight,
            "direction": "out",
        }));
    }

    // For multi-hop, expand further
    if hops > 1 {
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        visited.insert(entity_id.to_string());
        let mut frontier: Vec<String> = detail
            .inbound_relations
            .iter()
            .map(|r| r.from_id.clone())
            .chain(detail.outbound_relations.iter().map(|r| r.to_id.clone()))
            .collect();

        for _hop in 1..hops {
            if frontier.is_empty() {
                break;
            }
            let mut next_frontier: Vec<String> = Vec::new();
            for nid in &frontier {
                if !visited.insert(nid.clone()) {
                    continue;
                }
                if let Ok(sub_detail) = knowledge.get_entity_detail_local(nid) {
                    for r in &sub_detail.inbound_relations {
                        if !visited.contains(&r.from_id) {
                            neighbors.push(serde_json::json!({
                                "relation_id": r.id,
                                "source_id": r.from_id,
                                "source_name": find_entity_name(knowledge, &r.from_id),
                                "target_id": r.to_id,
                                "target_name": sub_detail.entity.name,
                                "relation_type": r.relation_type,
                                "label": r.label,
                                "weight": r.weight,
                                "distance": _hop + 1,
                            }));
                            next_frontier.push(r.from_id.clone());
                        }
                    }
                    for r in &sub_detail.outbound_relations {
                        if !visited.contains(&r.to_id) {
                            neighbors.push(serde_json::json!({
                                "relation_id": r.id,
                                "source_id": r.from_id,
                                "source_name": sub_detail.entity.name,
                                "target_id": r.to_id,
                                "target_name": find_entity_name(knowledge, &r.to_id),
                                "relation_type": r.relation_type,
                                "label": r.label,
                                "weight": r.weight,
                                "distance": _hop + 1,
                            }));
                            next_frontier.push(r.to_id.clone());
                        }
                    }
                }
            }
            frontier = next_frontier;
        }
    }

    Ok(serde_json::json!({
        "entity": entity_json,
        "neighbors": neighbors,
        "total_neighbors": neighbors.len(),
        "hops": hops,
    }))
}

fn find_entity_name(knowledge: &Arc<KnowledgeService>, entity_id: &str) -> String {
    knowledge
        .get_entity_detail_local(entity_id)
        .map(|d| d.entity.name)
        .unwrap_or_else(|_| entity_id.to_string())
}
