//! MCP resources 三工具执行：`list_resources`、`list_resource_templates`、`read_resource`。

use std::sync::Arc;

use rmcp::model::{
    ClientRequest, ListResourceTemplatesRequest, ListResourcesRequest, PaginatedRequestParams,
    ReadResourceRequest, ReadResourceRequestParams, ResourceContents, ServerResult,
};
use tokio::sync::watch;

use super::{
    Connection, McpCallOutcome, McpFailureKind, McpManager, McpToolSnapshot, ServerSnapshot,
};

impl McpManager {
    /// 列资源；`server` 省略时遍历所有声明 resources 的 server。
    pub async fn list_resources(
        &self,
        snapshot: &McpToolSnapshot,
        server: Option<&str>,
        cursor: Option<&str>,
        cancel: watch::Receiver<bool>,
    ) -> McpCallOutcome {
        self.list_resources_inner(snapshot, server, cursor, false, cancel)
            .await
    }

    /// 列资源模板。
    pub async fn list_resource_templates(
        &self,
        snapshot: &McpToolSnapshot,
        server: Option<&str>,
        cursor: Option<&str>,
        cancel: watch::Receiver<bool>,
    ) -> McpCallOutcome {
        self.list_resources_inner(snapshot, server, cursor, true, cancel)
            .await
    }

    async fn list_resources_inner(
        &self,
        snapshot: &McpToolSnapshot,
        server: Option<&str>,
        cursor: Option<&str>,
        templates: bool,
        cancel: watch::Receiver<bool>,
    ) -> McpCallOutcome {
        let Some(project) = snapshot.project() else {
            return McpCallOutcome::Failure {
                kind: McpFailureKind::InvalidArguments,
                text: "mcp resources are not available in this turn".to_string(),
            };
        };
        let servers: Vec<ServerSnapshot> = match server {
            Some(name) => match snapshot.server(name) {
                Some(server) if server.has_resources => vec![server.clone()],
                Some(_) => {
                    return McpCallOutcome::Failure {
                        kind: McpFailureKind::InvalidArguments,
                        text: format!("MCP server '{name}' does not expose resources"),
                    };
                }
                None => {
                    return McpCallOutcome::Failure {
                        kind: McpFailureKind::InvalidArguments,
                        text: format!("unknown MCP server '{name}'"),
                    };
                }
            },
            None => snapshot
                .servers
                .iter()
                .filter(|server| server.has_resources)
                .cloned()
                .collect(),
        };
        let mut items = Vec::new();
        let mut next_cursor = None;
        for server_snapshot in &servers {
            let Some(connection) = self.connection_for(snapshot, &server_snapshot.name).await
            else {
                continue;
            };
            let params = cursor.map(|cursor| {
                PaginatedRequestParams::default().with_cursor(Some(cursor.to_string()))
            });
            let request = if templates {
                ClientRequest::ListResourceTemplatesRequest(ListResourceTemplatesRequest {
                    method: Default::default(),
                    params,
                    extensions: Default::default(),
                })
            } else {
                ClientRequest::ListResourcesRequest(ListResourcesRequest {
                    method: Default::default(),
                    params,
                    extensions: Default::default(),
                })
            };
            let response = match self
                .send_cancellable(
                    project,
                    &server_snapshot.name,
                    &connection,
                    request,
                    cancel.clone(),
                    &format!("MCP server '{}'", server_snapshot.name),
                )
                .await
            {
                Ok(response) => response,
                Err(outcome) => return outcome,
            };
            match (templates, response) {
                (true, ServerResult::ListResourceTemplatesResult(result)) => {
                    for template in result.resource_templates {
                        items.push(serde_json::json!({
                            "server": server_snapshot.name,
                            "uriTemplate": template.uri_template,
                            "name": template.name,
                            "description": template.description,
                            "mimeType": template.mime_type,
                        }));
                    }
                    next_cursor = result.next_cursor;
                }
                (false, ServerResult::ListResourcesResult(result)) => {
                    for resource in result.resources {
                        items.push(serde_json::json!({
                            "server": server_snapshot.name,
                            "uri": resource.uri,
                            "name": resource.name,
                            "mimeType": resource.mime_type,
                            "description": resource.description,
                        }));
                    }
                    next_cursor = result.next_cursor;
                }
                (_, other) => {
                    return resource_error(
                        &server_snapshot.name,
                        format!("unexpected response: {other:?}"),
                    );
                }
            }
            if server.is_some() {
                break;
            }
        }
        let mut payload = serde_json::json!({ "resources": items });
        if templates {
            payload["kind"] = serde_json::json!("resource_templates");
        }
        if let Some(cursor) = next_cursor {
            payload["nextCursor"] = serde_json::json!(cursor);
        }
        McpCallOutcome::Success {
            text: payload.to_string(),
            images: Vec::new(),
            images_omitted: 0,
            truncated: false,
        }
    }

    async fn connection_for(
        &self,
        snapshot: &McpToolSnapshot,
        server: &str,
    ) -> Option<Arc<Connection>> {
        let project = snapshot.project()?;
        let slots = self.slots.lock().await;
        slots
            .get(&(project.to_path_buf(), server.to_string()))
            .and_then(|slot| slot.connection.clone())
    }

    /// 读取一个资源；内容按工具结果规则转换（图片归档）。
    pub async fn read_resource(
        &self,
        snapshot: &McpToolSnapshot,
        session_id: &str,
        server: &str,
        uri: &str,
        cancel: watch::Receiver<bool>,
    ) -> McpCallOutcome {
        let Some(project) = snapshot.project() else {
            return McpCallOutcome::Failure {
                kind: McpFailureKind::InvalidArguments,
                text: format!("unknown MCP server '{server}'"),
            };
        };
        let Some(connection) = self.connection_for(snapshot, server).await else {
            return McpCallOutcome::Failure {
                kind: McpFailureKind::InvalidArguments,
                text: format!("unknown MCP server '{server}'"),
            };
        };
        let request = ClientRequest::ReadResourceRequest(ReadResourceRequest::new(
            ReadResourceRequestParams::new(uri.to_string()),
        ));
        let response = match self
            .send_cancellable(
                project,
                server,
                &connection,
                request,
                cancel,
                &format!("MCP server '{server}'"),
            )
            .await
        {
            Ok(response) => response,
            Err(outcome) => return outcome,
        };
        match response {
            ServerResult::ReadResourceResult(result) => {
                let mut text = String::new();
                let mut images = Vec::new();
                let mut omitted = 0usize;
                for content in result.contents {
                    match content {
                        ResourceContents::TextResourceContents {
                            uri, text: body, ..
                        } => {
                            text.push_str(&format!("{uri}:\n{body}\n"));
                        }
                        ResourceContents::BlobResourceContents {
                            uri,
                            mime_type,
                            blob,
                            ..
                        } => {
                            let mime =
                                mime_type.unwrap_or_else(|| "application/octet-stream".into());
                            if mime.starts_with("image/") {
                                if images.len() >= super::super::result::MAX_RESULT_IMAGES {
                                    omitted = omitted.saturating_add(1);
                                } else {
                                    match super::super::result::archive_image(
                                        &blob, &mime, session_id,
                                    ) {
                                        Ok(image) => images.push(image),
                                        Err(error) => {
                                            omitted = omitted.saturating_add(1);
                                            tracing::warn!(
                                                target: "lx::mcp",
                                                %error,
                                                "resource image dropped"
                                            );
                                        }
                                    }
                                }
                            } else {
                                text.push_str(&format!(
                                    "[resource {uri} {mime}, {} bytes omitted]\n",
                                    super::super::result::decoded_len(&blob)
                                ));
                            }
                        }
                        _ => {}
                    }
                }
                if omitted > 0 {
                    text.push_str(&format!("[{omitted} more image(s) omitted]\n"));
                }
                let (text, truncated) = super::super::result::truncate_text(
                    text.trim_end(),
                    super::super::result::MAX_TOOL_RESULT_BYTES,
                );
                McpCallOutcome::Success {
                    text,
                    images,
                    images_omitted: omitted,
                    truncated,
                }
            }
            other => resource_error(server, format!("unexpected response: {other:?}")),
        }
    }
}

fn resource_error(server: &str, error: impl std::fmt::Display) -> McpCallOutcome {
    McpCallOutcome::Failure {
        kind: McpFailureKind::Server,
        text: format!("MCP server '{server}': {error}"),
    }
}
