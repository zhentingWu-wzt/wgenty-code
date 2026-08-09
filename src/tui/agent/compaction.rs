//! Micro/auto compaction policy lives in `agent::runtime`; auto-summary I/O is
//! `adapters::TuiCompactor`.

#[cfg(test)]
mod tests {
    // Prompt/parse checks that used to live next to do_auto_compact remain useful
    // as documentation of the summary JSON contract used by TuiCompactor.
    use crate::api::ChatMessage;

    #[test]
    fn test_compaction_prompt_includes_json_format() {
        let messages = [ChatMessage::system(
            "You are a conversation summary assistant for an AI coding agent. \
             Your task is to:\n\
             1. Summarize the conversation history, preserving key details: \
             project context, files modified, decisions made, bugs found, \
             commands executed, and any pending tasks.\n\
             2. Extract key memories from the conversation as structured JSON.\n\n\
             Output format — respond with a single JSON object (no markdown fences, no extra text):\n\
             {\n\
               \"summary\": \"<concise summary string>\",\n\
               \"memories\": [\n\
                 {\n\
                   \"type\": \"decision|error|preference|insight|knowledge|task\",\n\
                   \"content\": \"<what to remember>\",\n\
                   \"importance\": <0.0 to 1.0>\n\
                 }\n\
               ]\n\
             }\n\n\
             If there is nothing worth remembering, return an empty memories array.\n\
             Do NOT use any tools — just return the JSON as plain text.",
        )];
        let sys_content = messages[0].content.as_deref().unwrap();
        assert!(sys_content.contains("\"summary\""));
        assert!(sys_content.contains("\"memories\""));
        assert!(sys_content.contains("decision"));
        assert!(sys_content.contains("importance"));
    }

    #[test]
    fn test_parse_compaction_json_success() {
        let json_response = r#"{
            "summary": "The user asked about memory systems.",
            "memories": [
                {"type": "decision", "content": "Use Jaccard for dedup", "importance": 0.8},
                {"type": "knowledge", "content": "Project uses Rust", "importance": 0.6}
            ]
        }"#;
        let json: serde_json::Value = serde_json::from_str(json_response).unwrap();
        let summary = json.get("summary").and_then(|v| v.as_str()).unwrap();
        let memories = json.get("memories").and_then(|v| v.as_array()).unwrap();
        assert_eq!(summary, "The user asked about memory systems.");
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0]["type"].as_str().unwrap(), "decision");
    }

    #[test]
    fn test_parse_compaction_json_failure_graceful() {
        let bad_response = "This is just a plain text summary, not JSON at all.";
        let result = serde_json::from_str::<serde_json::Value>(bad_response);
        assert!(result.is_err());
    }
}
