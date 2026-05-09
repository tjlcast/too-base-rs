use assistant_message_parser::{AssistantMessageParser, ContentBlock, ParserError};

fn seeded_random(seed: u32) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        state as f64 / 0x1_0000_0000u64 as f64
    }
}

fn stream_chunks(parser: &mut AssistantMessageParser, message: &str) -> Vec<ContentBlock> {
    let mut result = Vec::new();
    let mut rng = seeded_random(42);
    let mut byte_indexes = message
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    byte_indexes.push(message.len());

    let mut char_index = 0;
    while char_index + 1 < byte_indexes.len() {
        let remaining = byte_indexes.len() - char_index - 1;
        let chunk_chars = remaining.min((rng() * 10.0) as usize + 1);
        let start = byte_indexes[char_index];
        let end = byte_indexes[char_index + chunk_chars];
        result = parser.process_chunk(&message[start..end]).unwrap();
        char_index += chunk_chars;
    }

    result
}

fn test_parser() -> AssistantMessageParser {
    let tools = [
        ("read_file", ["path", "start_line", "end_line"].as_slice()),
        (
            "write_to_file",
            ["path", "content", "line_count"].as_slice(),
        ),
        ("browser_action", [].as_slice()),
        ("search_files", ["regex", "path"].as_slice()),
        ("execute_command", ["command"].as_slice()),
        (
            "ask_followup_question",
            ["question", "follow_up"].as_slice(),
        ),
        ("new_rule", [].as_slice()),
    ];

    let tool_names = tools
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect::<Vec<_>>();
    let tool_param_names = tools
        .iter()
        .flat_map(|(_, params)| params.iter().map(|param| (*param).to_owned()))
        .collect::<Vec<_>>();

    AssistantMessageParser::new(Some(tool_names), Some(tool_param_names))
}

fn non_empty(block: &ContentBlock) -> bool {
    !matches!(block, ContentBlock::Text(text) if text.content.is_empty())
}

#[test]
fn accumulates_simple_text_chunk_by_chunk() {
    let mut parser = test_parser();
    let message = "Hello, this is a test.";
    let result = stream_chunks(&mut parser, message);

    assert_eq!(result.len(), 1);
    let ContentBlock::Text(text) = &result[0] else {
        panic!("expected text");
    };
    assert_eq!(text.content, message);
    assert!(text.partial);
}

#[test]
fn accumulates_multi_line_text_chunk_by_chunk() {
    let mut parser = test_parser();
    let message = "Line 1\nLine 2\nLine 3";
    let result = stream_chunks(&mut parser, message);

    let ContentBlock::Text(text) = &result[0] else {
        panic!("expected text");
    };
    assert_eq!(text.content, message);
    assert!(text.partial);
}

#[test]
fn parses_tool_use_with_parameter_streamed() {
    let mut parser = test_parser();
    let result = stream_chunks(
        &mut parser,
        "<read_file><path>src/file.ts</path></read_file>",
    )
    .into_iter()
    .filter(non_empty)
    .collect::<Vec<_>>();

    assert_eq!(result.len(), 1);
    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.name, "read_file");
    assert_eq!(tool.params.get("path").unwrap(), "src/file.ts");
    assert!(!tool.partial);
}

#[test]
fn marks_unclosed_tool_use_as_partial() {
    let mut parser = test_parser();
    let result = stream_chunks(&mut parser, "<read_file><path>src/file.ts</path>")
        .into_iter()
        .filter(non_empty)
        .collect::<Vec<_>>();

    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.params.get("path").unwrap(), "src/file.ts");
    assert!(tool.partial);
}

#[test]
fn handles_partial_parameter_in_tool_use() {
    let mut parser = test_parser();
    let result = stream_chunks(&mut parser, "<read_file><path>src/file")
        .into_iter()
        .filter(non_empty)
        .collect::<Vec<_>>();

    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.params.get("path").unwrap(), "src/file");
    assert!(tool.partial);
}

#[test]
fn handles_multiple_parameters_streamed() {
    let mut parser = test_parser();
    let result = stream_chunks(
        &mut parser,
        "<read_file><path>src/file.ts</path><start_line>10</start_line><end_line>20</end_line></read_file>",
    )
    .into_iter()
    .filter(non_empty)
    .collect::<Vec<_>>();

    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.params.get("path").unwrap(), "src/file.ts");
    assert_eq!(tool.params.get("start_line").unwrap(), "10");
    assert_eq!(tool.params.get("end_line").unwrap(), "20");
    assert!(!tool.partial);
}

#[test]
fn parses_text_followed_by_tool_use() {
    let mut parser = test_parser();
    let result = stream_chunks(
        &mut parser,
        "Text before tool <read_file><path>src/file.ts</path></read_file>",
    );

    assert_eq!(result.len(), 2);
    let ContentBlock::Text(text) = &result[0] else {
        panic!("expected text");
    };
    assert_eq!(text.content, "Text before tool");
    assert!(!text.partial);

    let ContentBlock::ToolUse(tool) = &result[1] else {
        panic!("expected tool");
    };
    assert_eq!(tool.params.get("path").unwrap(), "src/file.ts");
    assert!(!tool.partial);
}

#[test]
fn parses_tool_use_followed_by_text() {
    let mut parser = test_parser();
    let result = stream_chunks(
        &mut parser,
        "<read_file><path>src/file.ts</path></read_file>Text after tool",
    )
    .into_iter()
    .filter(non_empty)
    .collect::<Vec<_>>();

    assert_eq!(result.len(), 2);
    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.params.get("path").unwrap(), "src/file.ts");
    assert!(!tool.partial);

    let ContentBlock::Text(text) = &result[1] else {
        panic!("expected text");
    };
    assert_eq!(text.content, "Text after tool");
    assert!(text.partial);
}

#[test]
fn handles_lost_tool_prefix_and_text_chunk_generator() {
    let mut parser = test_parser();
    let result = stream_chunks(&mut parser, "<read_file")
        .into_iter()
        .filter(non_empty)
        .collect::<Vec<_>>();
    assert!(matches!(&result[0], ContentBlock::Text(text) if text.content == "<read_file"));

    parser.reset();
    let mut final_message = String::new();
    for ch in "abc<read_file><path>src/file.ts</path></read_file> Text after tool".chars() {
        parser.process_chunk(&ch.to_string()).unwrap();
        while let Some(chunk) = parser.next_text_chunk() {
            final_message.push_str(&chunk);
        }
    }
    assert_eq!(final_message, "abc Text after tool");

    parser.reset();
    final_message.clear();
    for ch in "abc<read_file><p".chars() {
        parser.process_chunk(&ch.to_string()).unwrap();
        while let Some(chunk) = parser.next_text_chunk() {
            final_message.push_str(&chunk);
        }
    }
    assert_eq!(final_message, "abc");

    parser.reset();
    final_message.clear();
    for ch in "abc<".chars() {
        parser.process_chunk(&ch.to_string()).unwrap();
        while let Some(chunk) = parser.next_text_chunk() {
            final_message.push_str(&chunk);
        }
    }
    assert_eq!(final_message, "abc");

    parser.reset();
    final_message.clear();
    for ch in "abc<read>daf".chars() {
        parser.process_chunk(&ch.to_string()).unwrap();
        while let Some(chunk) = parser.next_text_chunk() {
            final_message.push_str(&chunk);
        }
    }
    assert_eq!(final_message, "abc<read>daf");
}

#[test]
fn parses_multiple_tool_uses_separated_by_text() {
    let mut parser = test_parser();
    let result = stream_chunks(
        &mut parser,
        "First: <read_file><path>file1.ts</path></read_file>Second: <read_file><path>file2.ts</path></read_file>",
    );

    assert_eq!(result.len(), 4);
    assert!(matches!(&result[0], ContentBlock::Text(text) if text.content == "First:"));
    assert!(
        matches!(&result[1], ContentBlock::ToolUse(tool) if tool.params.get("path").unwrap() == "file1.ts")
    );
    assert!(matches!(&result[2], ContentBlock::Text(text) if text.content == "Second:"));
    assert!(
        matches!(&result[3], ContentBlock::ToolUse(tool) if tool.params.get("path").unwrap() == "file2.ts")
    );
}

#[test]
fn handles_write_to_file_content_containing_closing_tags() {
    let mut parser = test_parser();
    let message = r#"<write_to_file><path>src/file.ts</path><content>
function example() {
// This has XML-like content: </content>
return true;
}
</content><line_count>5</line_count></write_to_file>"#;

    let result = stream_chunks(&mut parser, message)
        .into_iter()
        .filter(non_empty)
        .collect::<Vec<_>>();

    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.params.get("path").unwrap(), "src/file.ts");
    assert_eq!(tool.params.get("line_count").unwrap(), "5");
    let content = tool.params.get("content").unwrap();
    assert!(content.contains("function example()"));
    assert!(content.contains("// This has XML-like content: </content>"));
    assert!(content.contains("return true;"));
    assert!(!tool.partial);
}

#[test]
fn handles_empty_messages() {
    let mut parser = test_parser();
    assert!(stream_chunks(&mut parser, "").is_empty());
}

#[test]
fn treats_malformed_tool_use_tags_as_plain_text() {
    let mut parser = test_parser();
    let message = "This has a <not_a_tool>malformed tag</not_a_tool>";
    let result = stream_chunks(&mut parser, message);

    assert_eq!(result.len(), 1);
    assert!(matches!(&result[0], ContentBlock::Text(text) if text.content == message));
}

#[test]
fn handles_tool_use_with_no_parameters() {
    let mut parser = test_parser();
    let result = stream_chunks(&mut parser, "<browser_action></browser_action>")
        .into_iter()
        .filter(non_empty)
        .collect::<Vec<_>>();

    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.name, "browser_action");
    assert!(tool.params.is_empty());
    assert!(!tool.partial);
}

#[test]
fn handles_xml_like_parameter_content() {
    let mut parser = test_parser();
    let result = stream_chunks(
        &mut parser,
        "<search_files><regex><div>.*</div></regex><path>src</path></search_files>",
    )
    .into_iter()
    .filter(non_empty)
    .collect::<Vec<_>>();

    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.params.get("regex").unwrap(), "<div>.*</div>");
    assert_eq!(tool.params.get("path").unwrap(), "src");
    assert!(!tool.partial);
}

#[test]
fn handles_consecutive_tool_uses() {
    let mut parser = test_parser();
    let result = stream_chunks(
        &mut parser,
        "<read_file><path>file1.ts</path></read_file><read_file><path>file2.ts</path></read_file>",
    )
    .into_iter()
    .filter(non_empty)
    .collect::<Vec<_>>();

    assert_eq!(result.len(), 2);
    assert!(
        matches!(&result[0], ContentBlock::ToolUse(tool) if tool.params.get("path").unwrap() == "file1.ts")
    );
    assert!(
        matches!(&result[1], ContentBlock::ToolUse(tool) if tool.params.get("path").unwrap() == "file2.ts")
    );
}

#[test]
fn trims_non_content_parameters() {
    let mut parser = test_parser();
    let result = stream_chunks(
        &mut parser,
        "<read_file><path>  src/file.ts  </path></read_file>",
    )
    .into_iter()
    .filter(non_empty)
    .collect::<Vec<_>>();

    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    assert_eq!(tool.params.get("path").unwrap(), "src/file.ts");
}

#[test]
fn keeps_multi_line_content_parameter() {
    let mut parser = test_parser();
    let message = "<write_to_file><path>file.ts</path><content>
line 1
line 2
line 3
</content><line_count>3</line_count></write_to_file>";
    let result = stream_chunks(&mut parser, message)
        .into_iter()
        .filter(non_empty)
        .collect::<Vec<_>>();

    let ContentBlock::ToolUse(tool) = &result[0] else {
        panic!("expected tool");
    };
    let content = tool.params.get("content").unwrap();
    assert!(content.contains("line 1"));
    assert!(content.contains("line 2"));
    assert!(content.contains("line 3"));
    assert_eq!(tool.params.get("line_count").unwrap(), "3");
}

#[test]
fn handles_complex_message_with_multiple_content_types() {
    let mut parser = test_parser();
    let message = r#"I'll help you with that task.

<read_file><path>src/index.ts</path></read_file>

Now let's modify the file:

<write_to_file><path>src/index.ts</path><content>
// Updated content
console.log("Hello world");
</content><line_count>2</line_count></write_to_file>

Let's run the code:

<execute_command><command>node src/index.ts</command></execute_command>"#;

    let result = stream_chunks(&mut parser, message);
    assert_eq!(result.len(), 6);
    assert!(
        matches!(&result[0], ContentBlock::Text(text) if text.content == "I'll help you with that task.")
    );
    assert!(matches!(&result[1], ContentBlock::ToolUse(tool) if tool.name == "read_file"));
    assert!(
        matches!(&result[2], ContentBlock::Text(text) if text.content.contains("Now let's modify the file:"))
    );
    assert!(matches!(&result[3], ContentBlock::ToolUse(tool) if tool.name == "write_to_file"));
    assert!(
        matches!(&result[4], ContentBlock::Text(text) if text.content.contains("Let's run the code:"))
    );
    assert!(matches!(&result[5], ContentBlock::ToolUse(tool) if tool.name == "execute_command"));
}

#[test]
fn errors_when_max_accumulator_size_exceeded() {
    let mut parser = test_parser();
    let large_message = "x".repeat(1024 * 1024 + 1);

    let error = parser.process_chunk(&large_message).unwrap_err();
    assert_eq!(error, ParserError::MessageTooLarge);
    assert_eq!(
        error.to_string(),
        "Assistant message exceeds maximum allowed size"
    );
}

#[test]
fn gracefully_handles_parameter_exceeding_max_length() {
    let mut parser = test_parser();
    let large_param_value = "x".repeat(1024 * 100 + 1);

    parser
        .process_chunk("<write_to_file><path>test.txt</path><content>")
        .unwrap();
    for chunk in large_param_value.as_bytes().chunks(1000) {
        parser
            .process_chunk(std::str::from_utf8(chunk).unwrap())
            .unwrap();
    }
    let result = parser
        .process_chunk("</content></write_to_file>After tool")
        .unwrap();

    let tool = result.iter().find_map(|block| match block {
        ContentBlock::ToolUse(tool) => Some(tool),
        ContentBlock::Text(_) => None,
    });
    assert_eq!(tool.unwrap().params.get("path").unwrap(), "test.txt");

    assert!(result.iter().any(
        |block| matches!(block, ContentBlock::Text(text) if text.content.contains("After tool"))
    ));
}

#[test]
fn finalizes_content_blocks() {
    let mut parser = test_parser();
    stream_chunks(&mut parser, "<read_file><path>src/file.ts");
    parser.finalize_content_blocks();

    assert!(parser
        .get_content_blocks()
        .iter()
        .all(|block| !block.partial()));
}

#[test]
fn handles_ask_followup_question() {
    let mut parser = test_parser();
    let message = r#"Example: Requesting to ask the user for the path to the frontend-config.json file
<ask_followup_question>
<question>What is the path to the frontend-config.json file?</question>
<follow_up>
<suggest>./src/frontend-config.json</suggest>
<suggest>./config/frontend-config.json</suggest>
<suggest>./frontend-config.json</suggest>
</follow_up>
</ask_followup_question>"#;
    stream_chunks(&mut parser, message);
    parser.finalize_content_blocks();

    assert!(parser
        .get_content_blocks()
        .iter()
        .all(|block| !block.partial()));
}

#[test]
fn thinking_tag_is_text() {
    let mut parser = test_parser();
    let result = stream_chunks(
        &mut parser,
        "<thinking>I'm thinking...</thinking> hello world",
    )
    .into_iter()
    .filter(non_empty)
    .collect::<Vec<_>>();

    assert_eq!(result.len(), 1);
    assert!(matches!(&result[0], ContentBlock::Text(text) if text.r#type == "text"));
}

#[test]
fn search_files_block_has_expected_shape() {
    let mut parser = test_parser();
    stream_chunks(
        &mut parser,
        "<search_files><regex><div>.*</div></regex><path>src</path></search_files>",
    );
    parser.finalize_content_blocks();
    let blocks = parser.get_content_blocks();

    assert!(blocks.iter().all(|block| !block.partial()));
    let ContentBlock::ToolUse(tool) = blocks.last().unwrap() else {
        panic!("expected tool");
    };
    assert_eq!(tool.name, "search_files");
    assert_eq!(tool.r#type, "tool_use");
    assert_eq!(tool.params.get("regex").unwrap(), "<div>.*</div>");
    assert_eq!(tool.params.get("path").unwrap(), "src");
}
