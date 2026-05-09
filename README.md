# Assistant Message Parser RS

Rust implementation of `src/examples/ai_chat_modular/experiments/assistant_message_parser.py`.

The crate is intentionally small and wasm-friendly:

- no filesystem, process, networking, thread, or timer APIs;
- no runtime dependencies;
- parser state is incremental, so callers can feed streaming chunks without reparsing the full message externally;
- library output uses owned Rust structs that can later be exposed with `wasm-bindgen` or another wasm boundary layer.

## Behavior

The parser emits two content block variants:

- `ContentBlock::Text(TextContent)` for normal assistant text;
- `ContentBlock::ToolUse(ToolUse)` for XML-like tool calls such as `<read_file><path>src/lib.rs</path></read_file>`.

It follows the Python implementation closely:

- text blocks are created while partial text streams in;
- recognized tool opening tags close the current text block and append a partial tool block immediately;
- parameter values are updated during streaming;
- non-`content` parameters are trimmed when their closing tag is seen;
- `content` parameters preserve internal newlines and strip only one leading and one trailing newline;
- `write_to_file` refreshes `content` from the last `</content>` so embedded `</content>` strings inside file content are preserved;
- messages larger than 1 MiB return `ParserError::MessageTooLarge`;
- parameter values larger than 100 KiB are abandoned gracefully, matching the Python parser's safe-state behavior.

## Usage

```rust
use assistant_message_parser::{AssistantMessageParser, ContentBlock};

let mut parser = AssistantMessageParser::default();
let blocks = parser
    .process_chunk("<read_file><args>src/main.rs</args></read_file>")
    .unwrap();

match &blocks[0] {
    ContentBlock::ToolUse(tool) => {
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.params.get("args").unwrap(), "src/main.rs");
    }
    ContentBlock::Text(_) => unreachable!(),
}
```

For custom tools:

```rust
use assistant_message_parser::AssistantMessageParser;

let parser = AssistantMessageParser::new(
    Some(vec!["read_file".into(), "write_to_file".into()]),
    Some(vec!["path".into(), "content".into()]),
);
```

## Classic Streaming Case

This mirrors the common LLM streaming flow: show normal assistant text as soon as it is safe to display, while separately collecting only completed tool calls.

```rust
use assistant_message_parser::{AssistantMessageParser, ContentBlock};

let mut parser = AssistantMessageParser::new(
    Some(vec!["read_file".into(), "search_files".into()]),
    Some(vec!["path".into(), "regex".into()]),
);

let response_chunks = [
    "I will inspect the file. ",
    "<read_file><path>src/main.rs</path></read_file>",
    " Then I will search. ",
    "<search_files><regex>fn main</regex><path>src</path></search_files>",
    " Done.",
];

let mut show_text = String::new();
let mut completed_tool_xml = Vec::new();
let mut seen_completed_tool_count = 0;

for chunk in response_chunks {
    let parsed_blocks = parser.process_chunk(chunk).unwrap();

    while let Some(text_chunk) = parser.next_text_chunk() {
        if !text_chunk.trim().is_empty() {
            // Send this delta to the UI.
            show_text.push_str(&text_chunk);
        }
    }

    let completed_tools = parsed_blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolUse(tool) if !tool.partial => Some(tool),
            _ => None,
        })
        .collect::<Vec<_>>();

    for tool in completed_tools.iter().skip(seen_completed_tool_count) {
        completed_tool_xml.push(tool.to_xml());
    }
    seen_completed_tool_count = completed_tools.len();
}

assert_eq!(show_text, "I will inspect the file. Then I will search. Done.");
assert_eq!(completed_tool_xml.len(), 2);
```

`next_text_chunk()` intentionally holds back text that may still be the prefix of a tool tag, so UI display does not briefly show `<read_file` before the parser can decide whether it is plain text or a real tool call.

## Test

```bash
cargo test
```
