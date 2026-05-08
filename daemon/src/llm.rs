use crate::ipc::CandidateItem;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct LlmClient {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    stream: bool,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
}

#[derive(Debug, Deserialize)]
struct LlmCandidates {
    candidates: Vec<LlmCandidate>,
}

#[derive(Debug, Deserialize)]
struct LlmCandidate {
    text: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Option<Vec<StreamChoice>>,
}

#[derive(Debug)]
pub enum LlmError {
    Http(reqwest::Error),
    NoContent,
    ParseFailed(String),
    RepetitionDetected,
    MaxTokensReached,
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Http(e) => write!(f, "HTTP error: {}", e),
            LlmError::NoContent => write!(f, "empty response from LLM"),
            LlmError::ParseFailed(reason) => write!(f, "parse failed: {}", reason),
            LlmError::RepetitionDetected => write!(f, "repetition detected"),
            LlmError::MaxTokensReached => write!(f, "max tokens reached"),
        }
    }
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        LlmError::Http(e)
    }
}

const MAX_CANDIDATES: usize = 9;
const MAX_CANDIDATE_LEN: usize = 500;
const INTER_CHUNK_TIMEOUT_SECS: u64 = 10;
const TOTAL_TIMEOUT_SECS: u64 = 120;
const REPETITION_WINDOW: usize = 20;
const REPETITION_THRESHOLD: usize = 3;

impl LlmClient {
    pub fn new(api_key: &str, model: &str, base_url: &str, timeout_secs: u64) -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(TOTAL_TIMEOUT_SECS))
            .build()
            .expect("Failed to create LLM HTTP client");

        Self {
            client,
            api_key: api_key.to_string(),
            model: model.to_string(),
            base_url: base_url.to_string(),
        }
    }

    pub async fn generate_candidates(
        &self,
        asr_text: &str,
        system_prompt: &str,
        thinking_mode: bool,
        max_tokens: u32,
    ) -> Result<Vec<CandidateItem>, LlmError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: asr_text.to_string(),
                },
            ],
            response_format: Some(ResponseFormat {
                format_type: "json_object".to_string(),
            }),
            stream: true,
            temperature: 0.3,
            max_tokens,
        };

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        debug!("LLM streaming request to {}", url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::ParseFailed(format!("API error ({}): {}", status, body)));
        }

        let mut content_buf = String::new();
        let mut reasoning_buf = String::new();
        let mut token_count: usize = 0;
        let mut last_activity = Instant::now();
        let start = Instant::now();
        let mut first_chunk_received = false;
        let mut repetition_checker = RepetitionChecker::new();

        let mut stream = response.bytes_stream();
        use futures_util::StreamExt;

        let mut sse_buffer = String::new();

        while let Some(chunk_result) = futures_util::StreamExt::next(&mut stream).await {
            let chunk = chunk_result?;
            let text = String::from_utf8_lossy(&chunk);
            sse_buffer.push_str(&text);

            while let Some(line_end) = sse_buffer.find('\n') {
                let line = sse_buffer[..line_end].trim().to_string();
                sse_buffer = sse_buffer[line_end + 1..].to_string();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if line == "data: [DONE]" {
                    debug!("SSE stream done");
                    return Self::finalize_content(&content_buf);
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    match serde_json::from_str::<StreamChunk>(data) {
                        Ok(chunk) => {
                            if let Some(choices) = chunk.choices {
                                if let Some(choice) = choices.first() {
                                    if !first_chunk_received {
                                        first_chunk_received = true;
                                        info!("LLM first chunk received");
                                    }
                                    last_activity = Instant::now();

                                    if let Some(ref reasoning) = choice.delta.reasoning_content {
                                        reasoning_buf.push_str(reasoning);
                                        token_count += 1;
                                        if token_count % 50 == 0 {
                                            debug!("Thinking... {} tokens", token_count);
                                        }
                                    }

                                    if let Some(ref content) = choice.delta.content {
                                        content_buf.push_str(content);
                                        token_count += 1;

                                        if repetition_checker.check(content) {
                                            warn!("Repetition detected at token {}", token_count);
                                            return Err(LlmError::RepetitionDetected);
                                        }
                                    }

                                    if token_count >= max_tokens as usize {
                                        warn!("Max tokens ({}) reached", max_tokens);
                                        return Err(LlmError::MaxTokensReached);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            debug!("Failed to parse SSE chunk: {} ({})", e, data);
                        }
                    }
                }
            }

            let elapsed_since_activity = last_activity.elapsed();
            let total_elapsed = start.elapsed();

            if first_chunk_received && elapsed_since_activity > Duration::from_secs(INTER_CHUNK_TIMEOUT_SECS) {
                warn!("Inter-chunk timeout ({}s)", elapsed_since_activity.as_secs());
                break;
            }
            if !first_chunk_received && total_elapsed > Duration::from_secs(15) {
                warn!("First chunk timeout ({}s)", total_elapsed.as_secs());
                break;
            }
            if total_elapsed > Duration::from_secs(TOTAL_TIMEOUT_SECS) {
                warn!("Total timeout ({}s)", total_elapsed.as_secs());
                break;
            }
        }

        if !reasoning_buf.is_empty() {
            debug!("LLM reasoning ({}chars): {}...", reasoning_buf.len(), &reasoning_buf[..reasoning_buf.len().min(200)]);
        }

        Self::finalize_content(&content_buf)
    }

    fn finalize_content(content_buf: &str) -> Result<Vec<CandidateItem>, LlmError> {
        if content_buf.trim().is_empty() {
            return Err(LlmError::NoContent);
        }
        debug!("LLM accumulated content: {}", content_buf);
        Ok(extract_candidates(content_buf))
    }
}

struct RepetitionChecker {
    recent_tokens: Vec<String>,
    repetition_count: usize,
    last_window_hash: Option<u64>,
}

impl RepetitionChecker {
    fn new() -> Self {
        Self {
            recent_tokens: Vec::with_capacity(REPETITION_WINDOW + 10),
            repetition_count: 0,
            last_window_hash: None,
        }
    }

    fn check(&mut self, token: &str) -> bool {
        self.recent_tokens.push(token.to_string());
        if self.recent_tokens.len() < REPETITION_WINDOW {
            return false;
        }

        let window: String = self.recent_tokens[self.recent_tokens.len() - REPETITION_WINDOW..].join("");
        let hash = Self::simple_hash(&window);

        if Some(hash) == self.last_window_hash {
            self.repetition_count += 1;
            if self.repetition_count >= REPETITION_THRESHOLD {
                return true;
            }
        } else {
            self.repetition_count = 0;
        }
        self.last_window_hash = Some(hash);
        false
    }

    fn simple_hash(s: &str) -> u64 {
        let mut hash: u64 = 5381;
        for b in s.bytes() {
            hash = hash.wrapping_mul(33).wrapping_add(b as u64);
        }
        hash
    }
}

fn extract_candidates(content: &str) -> Vec<CandidateItem> {
    let trimmed = content.trim();

    if trimmed.is_empty() {
        warn!("LLM returned empty content");
        return vec![CandidateItem {
            text: String::new(),
            source: Some("error_空响应".to_string()),
            confidence: None,
        }];
    }

    if let Ok(parsed) = try_parse_json(trimmed) {
        if !parsed.is_empty() {
            return parsed;
        }
    }

    if let Some(json_block) = extract_json_code_block(trimmed) {
        if let Ok(parsed) = try_parse_json(&json_block) {
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    if let Some(json_str) = extract_first_json_object(trimmed) {
        if let Ok(parsed) = try_parse_json(&json_str) {
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    warn!("LLM response is not valid JSON, using as plain text candidate");
    vec![CandidateItem {
        text: trimmed.chars().take(MAX_CANDIDATE_LEN).collect(),
        source: Some("error_解析失败".to_string()),
        confidence: None,
    }]
}

fn try_parse_json(content: &str) -> Result<Vec<CandidateItem>, serde_json::Error> {
    let parsed: LlmCandidates = serde_json::from_str(content)?;
    let mut candidates: Vec<CandidateItem> = parsed
        .candidates
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|c| CandidateItem {
            text: c.text.chars().take(MAX_CANDIDATE_LEN).collect(),
            source: Some(format!("llm_{}", c.label.unwrap_or_else(|| "variant".to_string()))),
            confidence: c.confidence,
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.confidence.unwrap_or(0.0)
            .partial_cmp(&a.confidence.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(candidates)
}

fn extract_json_code_block(content: &str) -> Option<String> {
    let start_marker = "```json";
    let start = content.find(start_marker)?;
    let after_start = start + start_marker.len();
    let end = content[after_start..].find("```")?;
    Some(content[after_start..after_start + end].trim().to_string())
}

fn extract_first_json_object(content: &str) -> Option<String> {
    let start = content.find('{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, c) in content[start..].char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            '\\' if in_string => escape = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(content[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
pub enum MockLlmScenario {
    Normal,
    MarkdownWrapped,
    BrokenJson,
    PlainText,
    Empty,
    Timeout,
    LongText,
}

impl MockLlmScenario {
    pub fn cycle(index: usize) -> Self {
        match index % 7 {
            0 => Self::Normal,
            1 => Self::MarkdownWrapped,
            2 => Self::BrokenJson,
            3 => Self::PlainText,
            4 => Self::Empty,
            5 => Self::Timeout,
            6 => Self::LongText,
            _ => Self::Normal,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Normal => "normal",
            Self::MarkdownWrapped => "markdown_wrapped",
            Self::BrokenJson => "broken_json",
            Self::PlainText => "plain_text",
            Self::Empty => "empty",
            Self::Timeout => "timeout",
            Self::LongText => "long_text",
        }
    }

    pub async fn mock_generate(&self, asr_text: &str, _timeout_secs: u64) -> Result<Vec<CandidateItem>, LlmError> {
        match self {
            Self::Normal => {
                let candidates = serde_json::json!({
                    "candidates": [
                        {"text": asr_text, "label": "纠错版", "confidence": 0.95},
                        {"text": format!("{}（已润色）", asr_text), "label": "润色版", "confidence": 0.8},
                        {"text": format!("{}（正式）", asr_text), "label": "正式版", "confidence": 0.6},
                    ]
                });
                let content = serde_json::to_string(&candidates).unwrap();
                Ok(extract_candidates(&content))
            }
            Self::MarkdownWrapped => {
                let inner = serde_json::json!({
                    "candidates": [
                        {"text": asr_text, "label": "纠错版", "confidence": 0.9},
                    ]
                });
                let content = format!("```json\n{}\n```", serde_json::to_string(&inner).unwrap());
                Ok(extract_candidates(&content))
            }
            Self::BrokenJson => {
                let content = r#"{"candidates":[{"text":"你好"#;
                Ok(extract_candidates(content))
            }
            Self::PlainText => {
                let content = "这是一段纯文本响应，不是JSON格式";
                Ok(extract_candidates(content))
            }
            Self::Empty => {
                let content = "";
                Ok(extract_candidates(content))
            }
            Self::Timeout => {
                tokio::time::sleep(Duration::from_secs(20)).await;
                Err(LlmError::ParseFailed("超时".to_string()))
            }
            Self::LongText => {
                let long = "这是一段非常长的文本用于测试候选列表在超长文本情况下的显示效果，".repeat(10);
                let candidates = serde_json::json!({
                    "candidates": [
                        {"text": long, "label": "纠错版", "confidence": 0.9},
                    ]
                });
                let content = serde_json::to_string(&candidates).unwrap();
                Ok(extract_candidates(&content))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_json() {
        let content = r#"{"candidates":[{"text":"你好","label":"纠错","confidence":0.9},{"text":"你好世界","label":"润色","confidence":0.7}]}"#;
        let result = extract_candidates(content);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, "你好");
        assert!(result[0].confidence.unwrap() > result[1].confidence.unwrap());
    }

    #[test]
    fn test_parse_markdown_code_block() {
        let content = "一些文字\n```json\n{\"candidates\":[{\"text\":\"测试\",\"label\":\"label\"}]}\n```\n更多文字";
        let result = extract_candidates(content);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "测试");
    }

    #[test]
    fn test_parse_broken_json_fallback() {
        let content = r#"{"candidates":[{"text":"你好"#;
        let result = extract_candidates(content);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source.as_deref(), Some("error_解析失败"));
    }

    #[test]
    fn test_parse_plain_text_fallback() {
        let content = "这不是JSON";
        let result = extract_candidates(content);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "这不是JSON");
    }

    #[test]
    fn test_parse_empty_returns_error_candidate() {
        let content = "";
        let result = extract_candidates(content);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source.as_deref(), Some("error_空响应"));
        assert!(result[0].text.is_empty());
    }

    #[test]
    fn test_max_candidates_limit() {
        let mut candidates_json = String::from("{\"candidates\":[");
        for i in 0..20 {
            if i > 0 {
                candidates_json.push(',');
            }
            candidates_json.push_str(&format!("{{\"text\":\"item{}\",\"label\":\"l{}\"}}", i, i));
        }
        candidates_json.push_str("]}");
        let result = extract_candidates(&candidates_json);
        assert_eq!(result.len(), MAX_CANDIDATES);
    }

    #[test]
    fn test_repetition_checker() {
        let mut checker = RepetitionChecker::new();
        for _ in 0..REPETITION_WINDOW + REPETITION_THRESHOLD - 1 {
            assert!(!checker.check("abc"));
        }
        assert!(checker.check("abc"));
    }

    #[test]
    fn test_no_false_positive_repetition() {
        let mut checker = RepetitionChecker::new();
        for i in 0..100 {
            assert!(!checker.check(&format!("token{}", i)));
        }
    }
}
