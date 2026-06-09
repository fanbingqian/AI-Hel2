use serde_json::{json, Value};

/// Returns the 5 Nexus knowledge tools as OpenAI function-calling JSON Schema.
/// Shared between MCP endpoint and openai_compatible.rs tool injection.
pub fn nexus_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "nexus_search",
                "description": "搜索本地知识库中的实体和关系。支持关键词全文搜索和命名空间过滤。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "搜索关键词"},
                        "namespace": {"type": "string", "description": "可选的命名空间过滤"},
                        "limit": {"type": "integer", "description": "返回结果数量上限，默认20"}
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "nexus_get_entity",
                "description": "获取指定实体的详细信息，包括属性、关系和来源。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entity_id": {"type": "string", "description": "实体ID或名称"}
                    },
                    "required": ["entity_id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "nexus_find_paths",
                "description": "查找两个实体之间的关联路径，用于发现隐藏的联系。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from": {"type": "string", "description": "起始实体ID或名称"},
                        "to": {"type": "string", "description": "目标实体ID或名称"},
                        "max_hops": {"type": "integer", "description": "最大跳数，默认4"}
                    },
                    "required": ["from", "to"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "nexus_get_neighbors",
                "description": "获取指定实体的邻居节点，展开N跳关系网络。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entity_id": {"type": "string", "description": "中心实体ID或名称"},
                        "hops": {"type": "integer", "description": "跳数，默认2，最大4"}
                    },
                    "required": ["entity_id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "nexus_get_map",
                "description": "获取知识图谱的全局概览，包括领域统计、子领域和桥接实体。",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
    ]
}

/// Execute a nexus tool call locally by calling the Nexus HTTP API (:18643).
/// Returns the tool result as a string.
pub async fn execute_nexus_tool(name: &str, arguments: &Value) -> Result<String, String> {
    let base = "http://127.0.0.1:18643";
    let client = reqwest::Client::new();

    match name {
        "nexus_search" => {
            let q = arguments["query"].as_str().unwrap_or("");
            let ns = arguments["namespace"].as_str().unwrap_or("");
            let limit = arguments["limit"].as_u64().unwrap_or(20);
            let url = format!("{base}/nexus/search?q={q}&namespace={ns}&limit={limit}");
            let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
            resp.text().await.map_err(|e| e.to_string())
        }
        "nexus_get_entity" => {
            let eid = arguments["entity_id"].as_str().unwrap_or("");
            let url = format!("{base}/nexus/entity/{eid}");
            let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
            resp.text().await.map_err(|e| e.to_string())
        }
        "nexus_find_paths" => {
            let from = arguments["from"].as_str().unwrap_or("");
            let to = arguments["to"].as_str().unwrap_or("");
            let max_hops = arguments["max_hops"].as_u64().unwrap_or(4);
            let url = format!("{base}/nexus/paths?from={from}&to={to}&max_hops={max_hops}");
            let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
            resp.text().await.map_err(|e| e.to_string())
        }
        "nexus_get_neighbors" => {
            let eid = arguments["entity_id"].as_str().unwrap_or("");
            let hops = arguments["hops"].as_u64().unwrap_or(2);
            let url = format!("{base}/nexus/neighbors/{eid}?hops={hops}");
            let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
            resp.text().await.map_err(|e| e.to_string())
        }
        "nexus_get_map" => {
            let url = format!("{base}/nexus/map");
            let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
            resp.text().await.map_err(|e| e.to_string())
        }
        _ => Err(format!("未知工具: {name}")),
    }
}
