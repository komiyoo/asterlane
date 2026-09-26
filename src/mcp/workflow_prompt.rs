use rmcp::model::{GetPromptResult, ListPromptsResult, Prompt, PromptMessage, Role};

const NAME: &str = "asterlane_tool_workflow";
const DESCRIPTION: &str = "Find, inspect, and call tools available to the current gateway key.";
const WORKFLOW: &str = "Use the tools visible to your current gateway key for this task:\n\
1. Call asterlane__search_tools with {\"query\": \"<what you need>\"}. If the result has next_cursor, pass that cursor to search the next page. Search results contain brief summaries, not complete argument schemas.\n\
2. Choose names returned by search and call asterlane__get_tools with {\"names\": [\"<returned canonical name>\"]} to read their descriptions and input_schema. You may request up to 10 names at once. Treat not_found as unavailable.\n\
3. Call asterlane__call_tool with {\"name\": \"<returned canonical name>\", \"arguments\": {}} using arguments that match the schema. For up to 10 independent calls, use asterlane__call_tools with {\"calls\": [{\"name\": \"<returned canonical name>\", \"arguments\": {}}]}. Batch results follow input order; inspect each result and retry only failed items when appropriate. Handle input_required if a tool requests more input.\n\
4. If a result includes a continuation cursor, call asterlane__fetch_result with {\"cursor\": \"<returned cursor>\"} to retrieve the remaining content.\n\
Only use names returned for this key; search and get_tools do not reveal tools outside its scope.";

pub(super) fn list_workflow_prompts() -> ListPromptsResult {
    ListPromptsResult::with_all_items(vec![Prompt::new(NAME, Some(DESCRIPTION), None)])
}

pub(super) fn get_workflow_prompt(name: &str) -> Option<GetPromptResult> {
    (name == NAME).then(|| {
        GetPromptResult::new(vec![PromptMessage::new_text(Role::User, WORKFLOW)])
            .with_description(DESCRIPTION)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_is_static_and_only_available_by_its_name() {
        let listed = list_workflow_prompts();
        assert_eq!(listed.prompts.len(), 1);
        assert_eq!(listed.prompts[0].name, NAME);
        assert!(listed.prompts[0].arguments.is_none());
        assert!(get_workflow_prompt("unknown").is_none());

        let fetched = get_workflow_prompt(NAME).expect("listed prompt can be fetched");
        assert_eq!(fetched.messages.len(), 1);
        let text = serde_json::to_value(&fetched.messages[0]).unwrap();
        let text = text["content"]["text"].as_str().unwrap();
        for tool in [
            "asterlane__search_tools",
            "asterlane__get_tools",
            "asterlane__call_tool",
            "asterlane__call_tools",
            "asterlane__fetch_result",
        ] {
            assert!(text.contains(tool), "missing {tool}");
        }
    }
}
