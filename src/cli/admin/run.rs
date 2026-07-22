use super::{
    AdminArgs, AdminCommand, DefaultsCommand, McpServersCommand, MetadataCommand, ProxyKeysCommand,
};
use crate::cli::client::{ApiClient, CliError, encode_path_segment, pretty};
use crate::cli::input::load_json_object;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

/// Executes an admin subcommand, prints its result, and returns an exit code.
pub async fn run_admin(args: AdminArgs) -> i32 {
    match execute(args).await {
        Ok(body) => {
            println!("{}", pretty(&body));
            0
        }
        Err(err) => err.report(),
    }
}

async fn execute(args: AdminArgs) -> Result<Value, CliError> {
    let client = ApiClient::new(args.server, &args.token_env)?;
    match args.command {
        AdminCommand::Stats => client.get("/admin/stats", &[]).await,
        AdminCommand::Resources => client.get("/admin/resources", &[]).await,
        AdminCommand::ProxyKeys { command } => run_proxy_keys(&client, command).await,
        AdminCommand::KeyPools => client.get("/admin/key-pools", &[]).await,
        AdminCommand::Presets => client.get("/admin/mcp-presets", &[]).await,
        AdminCommand::McpServers { command } => match command {
            None => client.get("/admin/mcp-servers", &[]).await,
            Some(McpServersCommand::Get { id }) => {
                client
                    .get(
                        &format!("/admin/mcp-servers/{}", encode_path_segment(&id)),
                        &[],
                    )
                    .await
            }
            Some(McpServersCommand::Probe { id }) => {
                client
                    .post_json(
                        &format!("/admin/mcp-servers/{}/probe", encode_path_segment(&id)),
                        &[],
                        &json!({}),
                    )
                    .await
            }
        },
        AdminCommand::Metadata { command } => run_metadata(&client, command).await,
        AdminCommand::Validate => client.get("/admin/config/validate", &[]).await,
        AdminCommand::Tools { filter } => Ok(filter_tools(
            client.get("/admin/tools", &[]).await?,
            filter.as_deref(),
        )?),
        AdminCommand::Events {
            tool,
            key,
            resource,
            limit,
            from,
            to,
        } => {
            let query = events_query(tool, key, resource, limit, from, to);
            client.get("/admin/events", &query).await
        }
        AdminCommand::SecurityEvents { resource, kind } => {
            let query: Vec<_> = [("resource_id", resource), ("kind", kind)]
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value)))
                .collect();
            client.get("/admin/security-events", &query).await
        }
        AdminCommand::Usage { group_by, from, to } => {
            client
                .get("/admin/usage", &usage_query(group_by, from, to))
                .await
        }
        AdminCommand::Defaults { command } => run_defaults(&client, command).await,
        AdminCommand::Invoke {
            tool,
            args,
            args_file,
            use_defaults,
            save_defaults,
        } => {
            let body = load_json_object(args, args_file)?.unwrap_or_else(|| json!({}));
            let query = [
                ("use_defaults", use_defaults.to_string()),
                ("save", save_defaults.to_string()),
            ];
            client
                .post_json(
                    &format!("/admin/tools/{}/invoke", encode_path_segment(&tool)),
                    &query,
                    &body,
                )
                .await
        }
    }
}

async fn run_proxy_keys(
    client: &ApiClient,
    command: Option<ProxyKeysCommand>,
) -> Result<Value, CliError> {
    match command {
        None => client.get("/admin/proxy-keys", &[]).await,
        Some(ProxyKeysCommand::Issue { id, expires_at }) => {
            let body = match expires_at {
                Some(ts) => json!({ "expires_at": ts }),
                None => json!({}),
            };
            let result = client
                .post_json(
                    &format!("/admin/proxy-keys/{}/token", encode_path_segment(&id)),
                    &[],
                    &body,
                )
                .await?;
            eprintln!("note: the token plaintext is shown only once; store it now");
            Ok(result)
        }
        Some(ProxyKeysCommand::RevokeToken { id }) => {
            client
                .delete(&format!(
                    "/admin/proxy-keys/{}/token",
                    encode_path_segment(&id)
                ))
                .await
        }
    }
}

async fn run_metadata(client: &ApiClient, command: MetadataCommand) -> Result<Value, CliError> {
    match command {
        MetadataCommand::List => client.get("/admin/tool-metadata", &[]).await,
        MetadataCommand::Get { tool } => {
            client
                .get(
                    &format!("/admin/tools/{}/metadata", encode_path_segment(&tool)),
                    &[],
                )
                .await
        }
        MetadataCommand::Set { tool, description } => {
            client
                .put_json(
                    &format!("/admin/tools/{}/metadata", encode_path_segment(&tool)),
                    &json!({ "description": description }),
                )
                .await
        }
        MetadataCommand::Rm { tool } => {
            client
                .delete(&format!(
                    "/admin/tools/{}/metadata",
                    encode_path_segment(&tool)
                ))
                .await
        }
    }
}

async fn run_defaults(client: &ApiClient, command: DefaultsCommand) -> Result<Value, CliError> {
    match command {
        DefaultsCommand::List => client.get("/admin/tool-defaults", &[]).await,
        DefaultsCommand::Get { tool } => {
            client
                .get(
                    &format!("/admin/tools/{}/defaults", encode_path_segment(&tool)),
                    &[],
                )
                .await
        }
        DefaultsCommand::Rm { tool } => {
            client
                .delete(&format!(
                    "/admin/tools/{}/defaults",
                    encode_path_segment(&tool)
                ))
                .await
        }
        DefaultsCommand::Set {
            tool,
            args,
            args_file,
            from_last_event,
        } => {
            let body = if from_last_event {
                last_event_args(client, &tool).await?
            } else {
                load_json_object(args, args_file)?.ok_or_else(|| {
                    anyhow!("defaults set requires one of --args, --args-file, --from-last-event")
                })?
            };
            client
                .put_json(
                    &format!("/admin/tools/{}/defaults", encode_path_segment(&tool)),
                    &body,
                )
                .await
        }
    }
}

async fn last_event_args(client: &ApiClient, tool: &str) -> Result<Value, CliError> {
    let events = client
        .get(
            "/admin/events",
            &[("tool_name", tool.to_string()), ("limit", "1".to_string())],
        )
        .await?;
    Ok(parse_last_event_args(&events, tool)?)
}

fn parse_last_event_args(events: &Value, tool: &str) -> Result<Value> {
    let row = events
        .as_array()
        .and_then(|rows| rows.first())
        .ok_or_else(|| anyhow!("no request events found for tool {tool}"))?;
    let raw = row
        .get("request_args")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow!(
                "last event for {tool} has no captured request args (is capture_payloads enabled?)"
            )
        })?;
    let value: Value = serde_json::from_str(raw).map_err(|_| {
        anyhow!(
            "captured request args for {tool} are truncated or not valid JSON \
             (capture_max_bytes cut them off); cannot use as defaults — pass --args explicitly"
        )
    })?;
    if !value.is_object() {
        bail!("captured request args for {tool} are not a JSON object");
    }
    Ok(value)
}

fn filter_tools(body: Value, filter: Option<&str>) -> Result<Value> {
    let Some(pattern) = filter else {
        return Ok(body);
    };
    let re =
        regex::Regex::new(pattern).with_context(|| format!("invalid --filter regex: {pattern}"))?;
    let tools: Vec<Value> = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| {
            t.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| re.is_match(name))
        })
        .collect();
    Ok(json!({ "total_count": tools.len(), "tools": tools }))
}

fn events_query(
    tool: Option<String>,
    key: Option<String>,
    resource: Option<String>,
    limit: Option<u32>,
    from: Option<String>,
    to: Option<String>,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(v) = tool {
        query.push(("tool_name", v));
    }
    if let Some(v) = key {
        query.push(("proxy_key_id", v));
    }
    if let Some(v) = resource {
        query.push(("resource_id", v));
    }
    if let Some(v) = limit {
        query.push(("limit", v.to_string()));
    }
    if let Some(v) = from {
        query.push(("from", v));
    }
    if let Some(v) = to {
        query.push(("to", v));
    }
    query
}

fn usage_query(
    group_by: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(v) = group_by {
        query.push(("group_by", v));
    }
    if let Some(v) = from {
        query.push(("from", v));
    }
    if let Some(v) = to {
        query.push(("to", v));
    }
    query
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_query_skips_unset_fields() {
        assert!(events_query(None, None, None, None, None, None).is_empty());
        let q = events_query(
            Some("t".into()),
            Some("k".into()),
            Some("r".into()),
            Some(3),
            Some("f".into()),
            Some("o".into()),
        );
        assert_eq!(
            q,
            vec![
                ("tool_name", "t".to_string()),
                ("proxy_key_id", "k".to_string()),
                ("resource_id", "r".to_string()),
                ("limit", "3".to_string()),
                ("from", "f".to_string()),
                ("to", "o".to_string()),
            ]
        );
    }

    #[test]
    fn usage_query_assembles_group_by() {
        let q = usage_query(Some("bucket".into()), None, Some("t".into()));
        assert_eq!(
            q,
            vec![("group_by", "bucket".to_string()), ("to", "t".to_string())]
        );
    }

    #[test]
    fn filter_tools_filters_by_name_regex() {
        let body = json!({
            "total_count": 3,
            "tools": [
                {"name": "search__exa__web_search_exa"},
                {"name": "docs__deepwiki__ask_question"},
                {"name": "search__tavily__web_search"},
            ]
        });
        let out = filter_tools(body.clone(), Some("^search__")).unwrap();
        assert_eq!(out["total_count"], 2);
        assert_eq!(out["tools"].as_array().unwrap().len(), 2);
        assert_eq!(filter_tools(body.clone(), None).unwrap(), body);
        assert!(filter_tools(body, Some("[")).is_err());
    }

    #[test]
    fn parse_last_event_args_happy_path() {
        let events = json!([{ "request_args": "{\"query\":\"rust\"}" }]);
        assert_eq!(
            parse_last_event_args(&events, "t").unwrap(),
            json!({"query":"rust"})
        );
    }

    #[test]
    fn parse_last_event_args_reports_clear_errors() {
        let no_rows = parse_last_event_args(&json!([]), "t").unwrap_err();
        assert!(no_rows.to_string().contains("no request events"));

        let no_capture = parse_last_event_args(&json!([{"request_args": null}]), "t").unwrap_err();
        assert!(no_capture.to_string().contains("no captured request args"));

        let truncated =
            parse_last_event_args(&json!([{"request_args": "{\"query\":\"ru"}]), "t").unwrap_err();
        assert!(truncated.to_string().contains("truncated"));

        let non_object = parse_last_event_args(&json!([{"request_args": "[1]"}]), "t").unwrap_err();
        assert!(non_object.to_string().contains("not a JSON object"));
    }
}
