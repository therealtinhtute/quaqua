//! OpenTelemetry GenAI JSONL span sink implementation.
//!
//! Provides [`JsonlSpanSink`], an append-only JSONL sink writing GenAI execution spans
//! to disk with deterministic attribute serialization following OpenTelemetry GenAI conventions.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::engine::telemetry::{
    GenAiSpan, SpanSink, TelemetryError, GEN_AI_AGENT_NAME, GEN_AI_AGENT_ROLE,
    GEN_AI_OPERATION_NAME, GEN_AI_REQUEST_MODEL, GEN_AI_SYSTEM, GEN_AI_USAGE_INPUT_TOKENS,
    GEN_AI_USAGE_OUTPUT_TOKENS,
};

/// Reserved keys for standard trace fields and GenAI attributes.
///
/// These keys are strictly governed by their corresponding typed fields on [`GenAiSpan`].
/// They are never emitted from [`GenAiSpan::attributes`] if their typed field is `None`.
const RESERVED_KEYS: &[&str] = &[
    "trace_id",
    "span_id",
    "start_time_ms",
    "end_time_ms",
    GEN_AI_SYSTEM,
    GEN_AI_OPERATION_NAME,
    GEN_AI_REQUEST_MODEL,
    GEN_AI_USAGE_INPUT_TOKENS,
    GEN_AI_USAGE_OUTPUT_TOKENS,
    GEN_AI_AGENT_NAME,
    GEN_AI_AGENT_ROLE,
];

/// An append-only OpenTelemetry GenAI JSONL span sink.
///
/// Records each [`GenAiSpan`] as a single-line JSON object. Keys are ordered
/// lexicographically using a [`BTreeMap`] for deterministic output.
#[derive(Debug, Clone)]
pub struct JsonlSpanSink {
    path: PathBuf,
}

impl JsonlSpanSink {
    /// Creates a new `JsonlSpanSink` writing to the specified destination path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns a reference to the configured trace destination path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Serializes a [`GenAiSpan`] to a deterministic single-line JSON string without trailing newline.
    ///
    /// - Standard keys use the `GEN_AI_*` constants.
    /// - Optional attributes are omitted when `None` (never written as `null`).
    /// - Keys are ordered lexicographically via [`BTreeMap`].
    /// - Typed fields take precedence over `span.attributes`: reserved keys in `span.attributes`
    ///   are never re-emitted if the corresponding typed field is `None`.
    /// - Any extra custom non-reserved attributes in `span.attributes` are merged at the top level.
    pub fn serialize_span(span: &GenAiSpan) -> Result<String, TelemetryError> {
        let mut map = BTreeMap::new();

        map.insert(
            "trace_id".to_string(),
            serde_json::Value::String(span.trace_id.clone()),
        );
        map.insert(
            "span_id".to_string(),
            serde_json::Value::String(span.span_id.clone()),
        );
        map.insert(
            "start_time_ms".to_string(),
            serde_json::Value::from(span.start_time_ms),
        );
        map.insert(
            "end_time_ms".to_string(),
            serde_json::Value::from(span.end_time_ms),
        );
        map.insert(
            GEN_AI_SYSTEM.to_string(),
            serde_json::Value::String(span.system.clone()),
        );
        map.insert(
            GEN_AI_OPERATION_NAME.to_string(),
            serde_json::Value::String(span.operation_name.clone()),
        );

        if let Some(model) = &span.request_model {
            map.insert(
                GEN_AI_REQUEST_MODEL.to_string(),
                serde_json::Value::String(model.clone()),
            );
        }
        if let Some(input_tokens) = span.input_tokens {
            map.insert(
                GEN_AI_USAGE_INPUT_TOKENS.to_string(),
                serde_json::Value::from(input_tokens),
            );
        }
        if let Some(output_tokens) = span.output_tokens {
            map.insert(
                GEN_AI_USAGE_OUTPUT_TOKENS.to_string(),
                serde_json::Value::from(output_tokens),
            );
        }
        if let Some(agent_name) = &span.agent_name {
            map.insert(
                GEN_AI_AGENT_NAME.to_string(),
                serde_json::Value::String(agent_name.clone()),
            );
        }
        if let Some(agent_role) = &span.agent_role {
            map.insert(
                GEN_AI_AGENT_ROLE.to_string(),
                serde_json::Value::String(agent_role.clone()),
            );
        }

        // Merge extra custom attributes from span.attributes.
        // Reserved keys are governed strictly by the typed fields above and are skipped here.
        for (k, v) in &span.attributes {
            if !RESERVED_KEYS.contains(&k.as_str()) {
                map.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
        }

        serde_json::to_string(&map)
            .map_err(|e| TelemetryError::SinkError(format!("JSON serialization failed: {e}")))
    }
}

impl Default for JsonlSpanSink {
    /// Constructs a sink targeting `<workspace_root>/runs/traces.jsonl`.
    fn default() -> Self {
        let path = crate::workspace::workspace_root()
            .join("runs")
            .join("traces.jsonl");
        Self::new(path)
    }
}

impl SpanSink for JsonlSpanSink {
    fn record_span(&self, span: &GenAiSpan) -> Result<(), TelemetryError> {
        let line = Self::serialize_span(span)?;

        // Ensure parent directory exists before opening file
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    TelemetryError::SinkError(format!(
                        "failed to create parent directory `{}`: {e}",
                        parent.display()
                    ))
                })?;
            }
        }

        // Open in append mode (O_APPEND semantics).
        // Open-append-write in one call is used for W1; cross-process locking is deferred to W2.
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| {
                TelemetryError::SinkError(format!(
                    "failed to open trace file `{}`: {e}",
                    self.path.display()
                ))
            })?;

        // Write the line buffer with trailing newline in a single write call
        // to ensure atomic append under O_APPEND semantics.
        let mut payload = line.into_bytes();
        payload.push(b'\n');

        file.write_all(&payload).map_err(|e| {
            TelemetryError::SinkError(format!(
                "failed to write to trace file `{}`: {e}",
                self.path.display()
            ))
        })?;

        file.flush().map_err(|e| {
            TelemetryError::SinkError(format!(
                "failed to flush trace file `{}`: {e}",
                self.path.display()
            ))
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    fn test_temp_file(prefix: &str) -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("quaqua_otel_test_{prefix}_{pid}_{n}.jsonl"))
    }

    const EXPECTED_GOLDEN_LINE: &str = r#"{"end_time_ms":1700000005000,"gen_ai.agent.name":"sude-e4","gen_ai.agent.role":"builder","gen_ai.operation.name":"agent_step","gen_ai.request.model":"claude-3-5-sonnet","gen_ai.system":"anthropic","gen_ai.usage.input_tokens":1024,"gen_ai.usage.output_tokens":256,"span_id":"span-golden-001","start_time_ms":1700000000000,"trace_id":"trace-golden-001"}"#;

    #[test]
    fn test_default_destination() {
        let sink = JsonlSpanSink::default();
        let expected = crate::workspace::workspace_root()
            .join("runs")
            .join("traces.jsonl");
        assert_eq!(sink.path(), expected.as_path());
    }

    #[test]
    fn test_golden_span_serialization() {
        let span = GenAiSpan::new(
            "trace-golden-001",
            "span-golden-001",
            "agent_step",
            "anthropic",
            1700000000000,
            1700000005000,
        )
        .with_model("claude-3-5-sonnet")
        .with_usage(1024, 256)
        .with_agent("sude-e4", "builder");

        // 1. Direct serialization exact bytes comparison
        let serialized = JsonlSpanSink::serialize_span(&span).expect("serialization failed");
        assert_eq!(serialized, EXPECTED_GOLDEN_LINE);

        // 2. File record_span exact bytes comparison
        let path = test_temp_file("golden");
        let sink = JsonlSpanSink::new(&path);
        sink.record_span(&span).expect("record_span failed");

        let file_contents = std::fs::read_to_string(&path).expect("read temp file failed");
        let _ = std::fs::remove_file(&path);

        assert_eq!(file_contents, format!("{EXPECTED_GOLDEN_LINE}\n"));
    }

    #[test]
    fn test_minimal_span_omits_none() {
        let span = GenAiSpan::new(
            "trace-min-001",
            "span-min-001",
            "agent_step",
            "anthropic",
            1700000000000,
            1700000005000,
        );

        let serialized = JsonlSpanSink::serialize_span(&span).expect("serialization failed");
        let expected = r#"{"end_time_ms":1700000005000,"gen_ai.operation.name":"agent_step","gen_ai.system":"anthropic","span_id":"span-min-001","start_time_ms":1700000000000,"trace_id":"trace-min-001"}"#;

        assert_eq!(serialized, expected);
        assert!(!serialized.contains("null"), "must not contain null values");
        assert!(!serialized.contains(GEN_AI_REQUEST_MODEL));
        assert!(!serialized.contains(GEN_AI_USAGE_INPUT_TOKENS));
        assert!(!serialized.contains(GEN_AI_USAGE_OUTPUT_TOKENS));
        assert!(!serialized.contains(GEN_AI_AGENT_NAME));
        assert!(!serialized.contains(GEN_AI_AGENT_ROLE));
    }

    #[test]
    fn test_record_span_append_twice() {
        let path = test_temp_file("append_twice");
        let sink = JsonlSpanSink::new(&path);

        let span1 = GenAiSpan::new("trace-1", "span-1", "step_1", "anthropic", 1000, 2000)
            .with_model("claude-3-5-sonnet")
            .with_usage(100, 50);

        let span2 = GenAiSpan::new("trace-1", "span-2", "step_2", "openai", 2000, 3000)
            .with_model("gpt-4o")
            .with_usage(200, 80);

        sink.record_span(&span1).expect("record span1 failed");
        sink.record_span(&span2).expect("record span2 failed");

        let content = std::fs::read_to_string(&path).expect("read file failed");
        let _ = std::fs::remove_file(&path);

        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "must have exactly two lines");

        let parsed1: serde_json::Value =
            serde_json::from_str(lines[0]).expect("line 1 must be valid json");
        assert_eq!(parsed1["trace_id"], "trace-1");
        assert_eq!(parsed1["span_id"], "span-1");
        assert_eq!(parsed1[GEN_AI_SYSTEM], "anthropic");
        assert_eq!(parsed1[GEN_AI_REQUEST_MODEL], "claude-3-5-sonnet");
        assert_eq!(parsed1[GEN_AI_USAGE_INPUT_TOKENS], 100);
        assert_eq!(parsed1[GEN_AI_USAGE_OUTPUT_TOKENS], 50);

        let parsed2: serde_json::Value =
            serde_json::from_str(lines[1]).expect("line 2 must be valid json");
        assert_eq!(parsed2["trace_id"], "trace-1");
        assert_eq!(parsed2["span_id"], "span-2");
        assert_eq!(parsed2[GEN_AI_SYSTEM], "openai");
        assert_eq!(parsed2[GEN_AI_REQUEST_MODEL], "gpt-4o");
        assert_eq!(parsed2[GEN_AI_USAGE_INPUT_TOKENS], 200);
        assert_eq!(parsed2[GEN_AI_USAGE_OUTPUT_TOKENS], 80);
    }

    #[test]
    fn test_record_span_creates_parent_directory() {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let nested_dir = std::env::temp_dir()
            .join(format!("quaqua_otel_nested_{pid}_{n}"))
            .join("sub1")
            .join("sub2");
        let path = nested_dir.join("traces.jsonl");

        assert!(
            !nested_dir.exists(),
            "directory must not exist prior to test"
        );

        let sink = JsonlSpanSink::new(&path);
        let span = GenAiSpan::new("t1", "s1", "op", "anthropic", 100, 200);

        sink.record_span(&span)
            .expect("record_span must create parent directory");

        assert!(
            nested_dir.exists(),
            "parent directory must have been created"
        );
        assert!(path.is_file(), "traces file must exist");

        let content = std::fs::read_to_string(&path).expect("reading file");
        assert!(content.contains("\"trace_id\":\"t1\""));

        let _ = std::fs::remove_dir_all(
            std::env::temp_dir().join(format!("quaqua_otel_nested_{pid}_{n}")),
        );
    }

    #[test]
    fn test_custom_attributes_merged() {
        let mut span = GenAiSpan::new("trace-c", "span-c", "op", "custom-sys", 1000, 2000);
        span.attributes
            .insert("custom.key1".to_string(), "val1".to_string());
        span.attributes
            .insert("custom.key2".to_string(), "val2".to_string());

        let serialized = JsonlSpanSink::serialize_span(&span).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&serialized).expect("json parse");

        assert_eq!(parsed["custom.key1"], "val1");
        assert_eq!(parsed["custom.key2"], "val2");
        assert_eq!(parsed["trace_id"], "trace-c");
    }

    #[test]
    fn test_sink_error_on_unwritable_path() {
        // Attempting to create a file inside a path where a component is a regular file
        let blocker_file = test_temp_file("blocker");
        std::fs::write(&blocker_file, b"not a directory").expect("create blocker file");

        let impossible_path = blocker_file
            .join("cannot_be_directory")
            .join("traces.jsonl");
        let sink = JsonlSpanSink::new(impossible_path);
        let span = GenAiSpan::new("t", "s", "op", "sys", 1, 2);

        let res = sink.record_span(&span);
        assert!(res.is_err(), "must fail when path cannot be created");
        match res.unwrap_err() {
            TelemetryError::SinkError(msg) => {
                assert!(!msg.is_empty());
            }
        }

        let _ = std::fs::remove_file(&blocker_file);
    }
    #[test]
    fn test_reserved_keys_in_attributes_ignored_when_typed_field_none() {
        let mut span = GenAiSpan::new("trace-r", "span-r", "op", "anthropic", 1000, 2000);
        // Inject reserved keys into attributes while typed fields remain None
        span.attributes
            .insert(GEN_AI_REQUEST_MODEL.to_string(), "ghost-model".to_string());
        span.attributes
            .insert(GEN_AI_USAGE_INPUT_TOKENS.to_string(), "999".to_string());
        span.attributes
            .insert(GEN_AI_USAGE_OUTPUT_TOKENS.to_string(), "555".to_string());
        span.attributes
            .insert(GEN_AI_AGENT_NAME.to_string(), "ghost-agent".to_string());
        span.attributes
            .insert(GEN_AI_AGENT_ROLE.to_string(), "ghost-role".to_string());
        span.attributes
            .insert("custom.valid".to_string(), "custom_val".to_string());

        assert!(span.request_model.is_none());
        assert!(span.input_tokens.is_none());
        assert!(span.output_tokens.is_none());
        assert!(span.agent_name.is_none());
        assert!(span.agent_role.is_none());

        let serialized = JsonlSpanSink::serialize_span(&span).expect("serialization failed");
        let parsed: serde_json::Value =
            serde_json::from_str(&serialized).expect("must parse as json");

        // Reserved keys must not be emitted when typed fields are None
        assert!(parsed.get(GEN_AI_REQUEST_MODEL).is_none());
        assert!(parsed.get(GEN_AI_USAGE_INPUT_TOKENS).is_none());
        assert!(parsed.get(GEN_AI_USAGE_OUTPUT_TOKENS).is_none());
        assert!(parsed.get(GEN_AI_AGENT_NAME).is_none());
        assert!(parsed.get(GEN_AI_AGENT_ROLE).is_none());

        // Custom non-reserved attributes must still be merged
        assert_eq!(parsed["custom.valid"], "custom_val");
    }

    #[test]
    fn test_record_span_atomic_single_buffer_newline() {
        let path = test_temp_file("atomic_buffer");
        let sink = JsonlSpanSink::new(&path);
        let span = GenAiSpan::new("t-atom", "s-atom", "op", "sys", 100, 200);

        sink.record_span(&span).expect("record_span failed");

        let raw_bytes = std::fs::read(&path).expect("read file bytes");
        let _ = std::fs::remove_file(&path);

        assert!(!raw_bytes.is_empty());
        assert_eq!(raw_bytes.last(), Some(&b'\n'));
        assert_eq!(raw_bytes.iter().filter(|&&b| b == b'\n').count(), 1);
    }
}
