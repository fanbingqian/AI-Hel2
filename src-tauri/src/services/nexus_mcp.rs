use tiny_http::{Header, Method, Request, Response, StatusCode};

use super::agents::nexus_tools;

/// Attach MCP routes to an existing tiny_http server loop.
/// Call this for each incoming request; returns Some(response) if handled.
pub fn handle_mcp_request(req: &mut Request) -> Option<Response<std::io::Cursor<Vec<u8>>>> {
    if req.url() != "/mcp" || req.method() != &Method::Post {
        return None;
    }

    let mut body = String::new();
    if req.as_reader().read_to_string(&mut body).is_err() {
        return Some(json_response(400, serde_json::json!({
            "jsonrpc": "2.0", "error": {"code": -32700, "message": "Parse error"}, "id": null
        })));
    }

    let v: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return Some(json_response(400, serde_json::json!({
                "jsonrpc": "2.0", "error": {"code": -32700, "message": "Parse error"}, "id": null
            })));
        }
    };

    let method = v["method"].as_str().unwrap_or("");
    let id = v.get("id").cloned().unwrap_or(serde_json::Value::Null);

    match method {
        "tools/list" => {
            let tools = nexus_tools::nexus_tool_definitions();
            Some(json_response(200, serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": tools }
            })))
        }
        "tools/call" => {
            let tool_name = v["params"]["name"].as_str().unwrap_or("");
            let tool_args = if v["params"]["arguments"].is_null() {
                serde_json::json!({})
            } else {
                v["params"]["arguments"].clone()
            };

            // Execute synchronously via blocking reqwest
            let result = match tool_name {
                "nexus_search" => {
                    let q = tool_args["query"].as_str().unwrap_or("");
                    let ns = tool_args["namespace"].as_str().unwrap_or("");
                    let limit = tool_args["limit"].as_u64().unwrap_or(20);
                    let url = format!("http://127.0.0.1:18643/nexus/search?q={q}&namespace={ns}&limit={limit}");
                    blocking_get(&url)
                }
                "nexus_get_entity" => {
                    let eid = tool_args["entity_id"].as_str().unwrap_or("");
                    let url = format!("http://127.0.0.1:18643/nexus/entity/{eid}");
                    blocking_get(&url)
                }
                "nexus_find_paths" => {
                    let from = tool_args["from"].as_str().unwrap_or("");
                    let to = tool_args["to"].as_str().unwrap_or("");
                    let max_hops = tool_args["max_hops"].as_u64().unwrap_or(4);
                    let url = format!("http://127.0.0.1:18643/nexus/paths?from={from}&to={to}&max_hops={max_hops}");
                    blocking_get(&url)
                }
                "nexus_get_neighbors" => {
                    let eid = tool_args["entity_id"].as_str().unwrap_or("");
                    let hops = tool_args["hops"].as_u64().unwrap_or(2);
                    let url = format!("http://127.0.0.1:18643/nexus/neighbors/{eid}?hops={hops}");
                    blocking_get(&url)
                }
                "nexus_get_map" => {
                    let url = "http://127.0.0.1:18643/nexus/map".to_string();
                    blocking_get(&url)
                }
                _ => Err(format!("Unknown tool: {tool_name}")),
            };

            match result {
                Ok(text) => {
                    let content: Vec<serde_json::Value> = vec![
                        serde_json::json!({"type": "text", "text": text})
                    ];
                    Some(json_response(200, serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "content": content }
                    })))
                }
                Err(e) => {
                    let content: Vec<serde_json::Value> = vec![
                        serde_json::json!({"type": "text", "text": format!("Error: {e}")})
                    ];
                    Some(json_response(200, serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "content": content, "isError": true }
                    })))
                }
            }
        }
        "initialize" => {
            Some(json_response(200, serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {"name": "nexus", "version": "0.1.0"},
                    "capabilities": {"tools": {}}
                }
            })))
        }
        _ => {
            Some(json_response(404, serde_json::json!({
                "jsonrpc": "2.0",
                "error": {"code": -32601, "message": format!("Method not found: {method}")},
                "id": id
            })))
        }
    }
}

fn json_response(status: u32, body: serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let body_str = serde_json::to_string(&body).unwrap_or_default();
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    Response::new(
        StatusCode(status.try_into().unwrap_or(200)),
        vec![header],
        std::io::Cursor::new(body_str.into_bytes()),
        None,
        None,
    )
}

fn blocking_get(url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    let resp = client.get(url).send().map_err(|e| e.to_string())?;
    resp.text().map_err(|e| e.to_string())
}
