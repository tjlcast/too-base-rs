use std::collections::HashMap;

pub const DEFAULT_TOOLS: &[(&str, &[&str])] = &[
    ("write_to_file", &["path", "content", "line_count"]),
    ("update_todo_list", &["todos"]),
    ("search_files", &["path", "regex", "file_pattern"]),
    (
        "search_and_replace",
        &[
            "path",
            "search",
            "replace",
            "start_line",
            "end_line",
            "use_regex",
            "ignore_case",
        ],
    ),
    ("read_file", &["args"]),
    ("list_files", &["path", "recursive"]),
    ("insert_content", &["path", "line", "content"]),
    ("execute_command", &["command", "cwd"]),
    ("attempt_completion", &["result"]),
    ("ask_followup_question", &["question", "follow_up"]),
    ("new_task", &["mode", "message"]),
    (
        "workflow_search",
        &[
            "q",
            "trigger",
            "complexity",
            "active_only",
            "page",
            "per_page",
        ],
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextContent {
    pub r#type: &'static str,
    pub content: String,
    pub partial: bool,
}

impl TextContent {
    pub fn new(content: String, partial: bool) -> Self {
        Self {
            r#type: "text",
            content,
            partial,
        }
    }

    pub fn to_xml(&self) -> &str {
        &self.content
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    pub r#type: &'static str,
    pub name: String,
    pub params: HashMap<String, String>,
    param_order: Vec<String>,
    pub partial: bool,
}

impl ToolUse {
    pub fn new(name: String, partial: bool) -> Self {
        Self {
            r#type: "tool_use",
            name,
            params: HashMap::new(),
            param_order: Vec::new(),
            partial,
        }
    }

    pub fn to_xml(&self) -> String {
        let mut xml = String::new();
        xml.push('<');
        xml.push_str(&self.name);
        xml.push('>');
        for key in &self.param_order {
            let Some(value) = self.params.get(key) else {
                continue;
            };
            xml.push('\n');
            xml.push('<');
            xml.push_str(key);
            xml.push('>');
            xml.push_str(value);
            xml.push_str("</");
            xml.push_str(key);
            xml.push('>');
        }
        xml.push('\n');
        xml.push_str("</");
        xml.push_str(&self.name);
        xml.push('>');
        xml
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentBlock {
    Text(TextContent),
    ToolUse(ToolUse),
}

impl ContentBlock {
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    pub fn is_tool_use(&self) -> bool {
        matches!(self, Self::ToolUse(_))
    }

    pub fn partial(&self) -> bool {
        match self {
            Self::Text(block) => block.partial,
            Self::ToolUse(block) => block.partial,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserError {
    MessageTooLarge,
}

impl std::fmt::Display for ParserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MessageTooLarge => write!(f, "Assistant message exceeds maximum allowed size"),
        }
    }
}

impl std::error::Error for ParserError {}

#[derive(Debug, Clone)]
pub struct AssistantMessageParser {
    tool_names: Vec<String>,
    tool_param_names: Vec<String>,
    tool_opening_tags: Vec<(String, String)>,
    param_opening_tags: Vec<(String, String)>,
    default_tool_prefixes: Vec<String>,
    max_accumulator_size: usize,
    max_param_length: usize,
    content_blocks: Vec<ContentBlock>,
    current_text_block_index: Option<usize>,
    current_text_content_start_index: usize,
    current_tool_block_index: Option<usize>,
    current_tool_use_start_index: usize,
    current_param_name: Option<String>,
    current_param_value_start_index: usize,
    accumulator: String,
    iter_idx: usize,
}

impl Default for AssistantMessageParser {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl AssistantMessageParser {
    pub const MAX_ACCUMULATOR_SIZE: usize = 1024 * 1024;
    pub const MAX_PARAM_LENGTH: usize = 1024 * 100;

    pub fn new(tool_names: Option<Vec<String>>, tool_param_names: Option<Vec<String>>) -> Self {
        let tool_names = tool_names.unwrap_or_else(default_tool_names);
        let tool_param_names = tool_param_names.unwrap_or_else(default_tool_param_names);
        let tool_opening_tags = tool_names
            .iter()
            .map(|name| (format!("<{}>", name), name.clone()))
            .collect();
        let param_opening_tags = tool_param_names
            .iter()
            .map(|name| (format!("<{}>", name), name.clone()))
            .collect();

        Self {
            tool_names,
            tool_param_names,
            tool_opening_tags,
            param_opening_tags,
            default_tool_prefixes: default_tool_prefixes(),
            max_accumulator_size: Self::MAX_ACCUMULATOR_SIZE,
            max_param_length: Self::MAX_PARAM_LENGTH,
            content_blocks: Vec::new(),
            current_text_block_index: None,
            current_text_content_start_index: 0,
            current_tool_block_index: None,
            current_tool_use_start_index: 0,
            current_param_name: None,
            current_param_value_start_index: 0,
            accumulator: String::new(),
            iter_idx: 0,
        }
    }

    pub fn reset(&mut self) {
        self.content_blocks.clear();
        self.current_text_block_index = None;
        self.current_text_content_start_index = 0;
        self.current_tool_block_index = None;
        self.current_tool_use_start_index = 0;
        self.current_param_name = None;
        self.current_param_value_start_index = 0;
        self.accumulator.clear();
        self.iter_idx = 0;
    }

    pub fn get_content_blocks(&self) -> Vec<ContentBlock> {
        self.content_blocks.clone()
    }

    pub fn process_chunk(&mut self, chunk: &str) -> Result<Vec<ContentBlock>, ParserError> {
        if self.accumulator.len() + chunk.len() > self.max_accumulator_size {
            return Err(ParserError::MessageTooLarge);
        }

        for ch in chunk.chars() {
            let current_position = self.accumulator.len();
            self.accumulator.push(ch);

            if self.current_tool_block_index.is_some() && self.current_param_name.is_some() {
                if self.process_current_param() {
                    continue;
                }
            }

            if self.current_tool_block_index.is_some() {
                self.process_current_tool();
                continue;
            }

            if !self.try_start_tool_use() {
                self.process_text(current_position);
            }
        }

        Ok(self.get_content_blocks())
    }

    pub fn finalize_content_blocks(&mut self) {
        for block in &mut self.content_blocks {
            match block {
                ContentBlock::Text(text) => {
                    text.partial = false;
                    text.content = text.content.trim().to_owned();
                }
                ContentBlock::ToolUse(tool) => {
                    tool.partial = false;
                }
            }
        }
    }

    pub fn is_last_block_text_surely(&self, _content: &str) -> bool {
        let Some(ContentBlock::Text(text)) = self.content_blocks.last() else {
            return false;
        };

        !self
            .default_tool_prefixes
            .iter()
            .any(|prefix| text.content.ends_with(prefix))
    }

    pub fn next_text_chunk(&mut self) -> Option<String> {
        let text_content = self
            .content_blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.content.as_str()),
                ContentBlock::ToolUse(_) => None,
            })
            .collect::<Vec<_>>()
            .join(" ");

        if self.iter_idx >= text_content.len() {
            return None;
        }

        if let Some((_, start_index)) = self.check_tool_prefix(&text_content) {
            if self.iter_idx < start_index {
                let chunk = text_content[self.iter_idx..start_index].to_owned();
                self.iter_idx = start_index;
                Some(chunk)
            } else {
                None
            }
        } else {
            let chunk = text_content[self.iter_idx..].to_owned();
            self.iter_idx = text_content.len();
            Some(chunk)
        }
    }

    fn process_current_param(&mut self) -> bool {
        let param_name = self.current_param_name.clone().expect("checked above");
        let current_param_value = &self.accumulator[self.current_param_value_start_index..];

        if current_param_value.len() > self.max_param_length {
            self.current_param_name = None;
            self.current_param_value_start_index = 0;
            return false;
        }

        let closing_tag = format!("</{}>", param_name);
        if current_param_value.ends_with(&closing_tag) {
            let raw_value = &current_param_value[..current_param_value.len() - closing_tag.len()];
            let value = normalize_param_value(&param_name, raw_value);
            self.set_current_tool_param(&param_name, value);
            self.current_param_name = None;
        } else {
            self.set_current_tool_param(&param_name, current_param_value.to_owned());
        }

        true
    }

    fn process_current_tool(&mut self) {
        let Some(tool_index) = self.current_tool_block_index else {
            return;
        };
        let tool_name = match &self.content_blocks[tool_index] {
            ContentBlock::ToolUse(tool) => tool.name.clone(),
            ContentBlock::Text(_) => return,
        };
        let current_tool_value = &self.accumulator[self.current_tool_use_start_index..];
        let closing_tag = format!("</{}>", tool_name);

        if current_tool_value.ends_with(&closing_tag) {
            if let ContentBlock::ToolUse(tool) = &mut self.content_blocks[tool_index] {
                tool.partial = false;
            }
            self.current_tool_block_index = None;
            return;
        }

        for (opening_tag, param_name) in &self.param_opening_tags {
            if self.accumulator.ends_with(opening_tag) && self.is_valid_param_name(param_name) {
                self.current_param_name = Some(param_name.clone());
                self.current_param_value_start_index = self.accumulator.len();
                break;
            }
        }

        if tool_name == "write_to_file" && self.accumulator.ends_with("</content>") {
            self.refresh_write_to_file_content(tool_index);
        }
    }

    fn try_start_tool_use(&mut self) -> bool {
        let Some((opening_tag, tool_name)) = self
            .tool_opening_tags
            .iter()
            .find(|(tag, name)| self.accumulator.ends_with(tag) && self.is_valid_tool_name(name))
            .cloned()
        else {
            return false;
        };

        let tool = ToolUse::new(tool_name.clone(), true);
        self.content_blocks.push(ContentBlock::ToolUse(tool));
        self.current_tool_block_index = Some(self.content_blocks.len() - 1);
        self.current_tool_use_start_index = self.accumulator.len();

        if let Some(text_index) = self.current_text_block_index.take() {
            if let ContentBlock::Text(text) = &mut self.content_blocks[text_index] {
                text.partial = false;
                let partial_tag = &opening_tag[..opening_tag.len() - 1];
                let keep_len = text.content.len().saturating_sub(partial_tag.len());
                text.content = text.content[..keep_len].trim().to_owned();
            }
        }

        true
    }

    fn process_text(&mut self, current_position: usize) {
        if let Some(text_index) = self.current_text_block_index {
            if let ContentBlock::Text(text) = &mut self.content_blocks[text_index] {
                text.content = self.accumulator[self.current_text_content_start_index..]
                    .trim()
                    .to_owned();
            }
            return;
        }

        self.current_text_content_start_index = current_position;
        let text = TextContent::new(
            self.accumulator[self.current_text_content_start_index..]
                .trim()
                .to_owned(),
            true,
        );
        self.content_blocks.push(ContentBlock::Text(text));
        self.current_text_block_index = Some(self.content_blocks.len() - 1);
    }

    fn set_current_tool_param(&mut self, name: &str, value: String) {
        let Some(tool_index) = self.current_tool_block_index else {
            return;
        };
        if let ContentBlock::ToolUse(tool) = &mut self.content_blocks[tool_index] {
            if !tool.params.contains_key(name) {
                tool.param_order.push(name.to_owned());
            }
            tool.params.insert(name.to_owned(), value);
        }
    }

    fn refresh_write_to_file_content(&mut self, tool_index: usize) {
        let tool_content = &self.accumulator[self.current_tool_use_start_index..];
        let start_tag = "<content>";
        let end_tag = "</content>";
        let Some(start) = tool_content
            .find(start_tag)
            .map(|index| index + start_tag.len())
        else {
            return;
        };
        let Some(end) = tool_content.rfind(end_tag) else {
            return;
        };

        if end <= start {
            return;
        }

        let value = normalize_param_value("content", &tool_content[start..end]);
        if let ContentBlock::ToolUse(tool) = &mut self.content_blocks[tool_index] {
            if !tool.params.contains_key("content") {
                tool.param_order.push("content".to_owned());
            }
            tool.params.insert("content".to_owned(), value);
        }
    }

    fn check_tool_prefix<'a>(&'a self, content: &str) -> Option<(&'a str, usize)> {
        for prefix in &self.default_tool_prefixes {
            if content.ends_with(prefix) {
                return Some((prefix.as_str(), content.len() - prefix.len()));
            }
        }
        None
    }

    fn is_valid_tool_name(&self, name: &str) -> bool {
        self.tool_names.iter().any(|known| known == name)
    }

    fn is_valid_param_name(&self, name: &str) -> bool {
        self.tool_param_names.iter().any(|known| known == name)
    }
}

pub fn default_tool_names() -> Vec<String> {
    DEFAULT_TOOLS
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect()
}

pub fn default_tool_param_names() -> Vec<String> {
    DEFAULT_TOOLS
        .iter()
        .flat_map(|(_, params)| params.iter().map(|param| (*param).to_owned()))
        .collect()
}

pub fn default_tool_prefixes() -> Vec<String> {
    let mut prefixes = Vec::new();
    for (tool_name, _) in DEFAULT_TOOLS {
        for end in 0..=tool_name.len() {
            prefixes.push(format!("<{}", &tool_name[..end]));
        }
    }
    prefixes.sort();
    prefixes
}

fn normalize_param_value(param_name: &str, raw_value: &str) -> String {
    if param_name == "content" {
        let mut value = raw_value;
        if let Some(stripped) = value.strip_prefix('\n') {
            value = stripped;
        }
        if let Some(stripped) = value.strip_suffix('\n') {
            value = stripped;
        }
        value.to_owned()
    } else {
        raw_value.trim().to_owned()
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{AssistantMessageParser, ContentBlock, ParserError};
    use js_sys::{Array, Object, Reflect};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(js_name = AssistantMessageParser)]
    pub struct WasmAssistantMessageParser {
        parser: AssistantMessageParser,
    }

    #[wasm_bindgen(js_class = AssistantMessageParser)]
    impl WasmAssistantMessageParser {
        #[wasm_bindgen(constructor)]
        pub fn new(tool_names: JsValue, tool_param_names: JsValue) -> Result<Self, JsValue> {
            let tool_names = optional_string_array(&tool_names, "toolNames")?;
            let tool_param_names = optional_string_array(&tool_param_names, "toolParamNames")?;

            Ok(Self {
                parser: AssistantMessageParser::new(tool_names, tool_param_names),
            })
        }

        pub fn default() -> Self {
            Self {
                parser: AssistantMessageParser::default(),
            }
        }

        pub fn reset(&mut self) {
            self.parser.reset();
        }

        #[wasm_bindgen(js_name = processChunk)]
        pub fn process_chunk(&mut self, chunk: &str) -> Result<JsValue, JsValue> {
            self.parser
                .process_chunk(chunk)
                .map(blocks_to_js)
                .map_err(parser_error_to_js)
        }

        #[wasm_bindgen(js_name = getContentBlocks)]
        pub fn get_content_blocks(&self) -> JsValue {
            blocks_to_js(self.parser.get_content_blocks())
        }

        #[wasm_bindgen(js_name = finalizeContentBlocks)]
        pub fn finalize_content_blocks(&mut self) {
            self.parser.finalize_content_blocks();
        }

        #[wasm_bindgen(js_name = nextTextChunk)]
        pub fn next_text_chunk(&mut self) -> Option<String> {
            self.parser.next_text_chunk()
        }
    }

    fn optional_string_array(value: &JsValue, name: &str) -> Result<Option<Vec<String>>, JsValue> {
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }

        if !Array::is_array(value) {
            return Err(js_error(&format!("{name} must be an array of strings")));
        }

        let array = Array::from(value);
        let mut strings = Vec::with_capacity(array.length() as usize);
        for item in array.iter() {
            let Some(item) = item.as_string() else {
                return Err(js_error(&format!("{name} must be an array of strings")));
            };
            strings.push(item);
        }

        Ok(Some(strings))
    }

    fn blocks_to_js(blocks: Vec<ContentBlock>) -> JsValue {
        let array = Array::new();
        for block in blocks {
            array.push(&block_to_js(block));
        }
        array.into()
    }

    fn block_to_js(block: ContentBlock) -> JsValue {
        let object = Object::new();
        match block {
            ContentBlock::Text(text) => {
                set(&object, "type", JsValue::from_str(text.r#type));
                set(&object, "content", JsValue::from_str(&text.content));
                set(&object, "partial", JsValue::from_bool(text.partial));
            }
            ContentBlock::ToolUse(tool) => {
                set(&object, "type", JsValue::from_str(tool.r#type));
                set(&object, "name", JsValue::from_str(&tool.name));
                set(&object, "partial", JsValue::from_bool(tool.partial));
                set(&object, "xml", JsValue::from_str(&tool.to_xml()));

                let params = Object::new();
                for (key, value) in tool.params {
                    set(&params, &key, JsValue::from_str(&value));
                }
                set(&object, "params", params.into());
            }
        }
        object.into()
    }

    fn set(object: &Object, key: &str, value: JsValue) {
        Reflect::set(object, &JsValue::from_str(key), &value).expect("setting object property");
    }

    fn parser_error_to_js(error: ParserError) -> JsValue {
        js_error(&error.to_string())
    }

    fn js_error(message: &str) -> JsValue {
        js_sys::Error::new(message).into()
    }
}
