use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::client::{ApiClient, CliError, encode_path_segment};
use super::input::load_json_object;
use super::output::{emit, resolve_cli_format};

const DEFAULT_TOKEN_ENV: &str = "ASTERLANE_KEY";

#[derive(Debug, clap::Args)]
pub struct ToolsArgs {
    #[arg(long, global = true)]
    pub server: Option<String>,
    #[arg(long, default_value = DEFAULT_TOKEN_ENV, global = true)]
    pub token_env: String,
    #[arg(long, short, global = true)]
    pub format: Option<String>,
    #[command(subcommand)]
    pub command: ToolsCommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum ToolsCommand {
    List {
        #[arg(long)]
        include: Option<String>,
        #[arg(long)]
        exclude: Option<String>,
        #[arg(long)]
        domain: Option<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        tool: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        cursor: Option<usize>,
    },
    Search {
        query: String,
    },
    Call {
        name: String,
        #[arg(long, conflicts_with = "args_file")]
        args: Option<String>,
        #[arg(long)]
        args_file: Option<PathBuf>,
    },
}

pub async fn run_tools(args: ToolsArgs) -> i32 {
    let format = match resolve_cli_format(args.format.as_deref()) {
        Ok(format) => format,
        Err(error) => return CliError::from(error).report(),
    };
    match execute(args).await {
        Ok(body) => {
            emit(&body, format);
            0
        }
        Err(error) => error.report(),
    }
}

async fn execute(args: ToolsArgs) -> Result<Value, CliError> {
    let client = ApiClient::new(args.server, &args.token_env)?;
    match args.command {
        ToolsCommand::List {
            include,
            exclude,
            domain,
            provider,
            tool,
            limit,
            cursor,
        } => {
            client
                .get(
                    "/v1/tools",
                    &list_query(include, exclude, domain, provider, tool, limit, cursor),
                )
                .await
        }
        ToolsCommand::Search { query } => {
            let body = client
                .post_json(
                    "/v1/tools/asterlane__search_tools/invoke",
                    &[("format", "json".to_string())],
                    &json!({"query": query}),
                )
                .await?;
            Ok(normalize_search_result(body)?)
        }
        ToolsCommand::Call {
            name,
            args,
            args_file,
        } => {
            let body = load_json_object(args, args_file)?.unwrap_or_else(|| json!({}));
            client
                .post_json(
                    &format!("/v1/tools/{}/invoke", encode_path_segment(&name)),
                    &[("format", "json".to_string())],
                    &body,
                )
                .await
        }
    }
}

fn list_query(
    include: Option<String>,
    exclude: Option<String>,
    domain: Option<String>,
    provider: Option<String>,
    tool: Option<String>,
    limit: Option<usize>,
    cursor: Option<usize>,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    for (key, value) in [
        ("include", include),
        ("exclude", exclude),
        ("domain", domain),
        ("provider", provider),
        ("tool", tool),
    ] {
        if let Some(value) = value {
            query.push((key, value));
        }
    }
    if let Some(value) = limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = cursor {
        query.push(("cursor", value.to_string()));
    }
    query
}

fn normalize_search_result(body: Value) -> Result<Value> {
    let text = body
        .pointer("/content/0/Text")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("unexpected search response: missing content[0].Text"))?;
    if body.get("is_error").and_then(Value::as_bool) == Some(true) {
        bail!("tool search failed: {text}");
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|error| anyhow!("unexpected search response JSON: {error}"))?;
    if !value.is_array() {
        bail!("unexpected search response: result is not an array");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use serde_json::json;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        tools: ToolsArgs,
    }

    fn parse(args: &[&str]) -> ToolsArgs {
        TestCli::try_parse_from(std::iter::once("tools").chain(args.iter().copied()))
            .unwrap()
            .tools
    }

    fn parse_err(args: &[&str]) {
        TestCli::try_parse_from(std::iter::once("tools").chain(args.iter().copied()))
            .expect_err("args should fail to parse");
    }

    #[test]
    fn parses_list_call_search_and_global_flags() {
        assert!(matches!(
            parse(&["list"]).command,
            ToolsCommand::List { .. }
        ));
        assert!(matches!(
            parse(&["search", "web search"]).command,
            ToolsCommand::Search { .. }
        ));
        let args = parse(&["call", "search", "--args", "{}", "-f", "yaml"]);
        assert_eq!(args.format.as_deref(), Some("yaml"));
        assert!(matches!(args.command, ToolsCommand::Call { .. }));

        let args = parse(&[
            "list",
            "--server",
            "http://gw:9000",
            "--token-env",
            "MY_TOKEN",
        ]);
        assert_eq!(args.server.as_deref(), Some("http://gw:9000"));
        assert_eq!(args.token_env, "MY_TOKEN");

        parse_err(&["call", "search", "--args", "{}", "--args-file", "a.json"]);
    }

    #[test]
    fn list_query_skips_none_and_keeps_all_filters() {
        let query = list_query(
            Some("a".into()),
            Some("x".into()),
            Some("d".into()),
            Some("p".into()),
            Some("t".into()),
            Some(20),
            Some(40),
        );
        assert_eq!(
            query,
            vec![
                ("include", "a".into()),
                ("exclude", "x".into()),
                ("domain", "d".into()),
                ("provider", "p".into()),
                ("tool", "t".into()),
                ("limit", "20".into()),
                ("cursor", "40".into()),
            ]
        );
    }

    #[test]
    fn search_result_extracts_meta_tool_json() {
        let body = json!({"content": [{"Text": "[{\"name\":\"search\"}]"}], "is_error": false});
        assert_eq!(
            normalize_search_result(body).unwrap(),
            json!([{"name": "search"}])
        );
    }

    #[test]
    fn search_result_rejects_invalid_gateway_responses() {
        for body in [
            json!({"content": [{"Text": "failure"}], "is_error": true}),
            json!({"content": [{}], "is_error": false}),
            json!({"content": [{"Text": 1}], "is_error": false}),
            json!({"content": [{"Text": "not json"}], "is_error": false}),
            json!({"content": [{"Text": "{}"}], "is_error": false}),
        ] {
            assert!(normalize_search_result(body).is_err());
        }
    }
}
