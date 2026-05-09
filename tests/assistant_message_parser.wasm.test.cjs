const assert = require("node:assert/strict");
const { AssistantMessageParser } = require("../pkg/assistant_message_parser.js");

function seededRandom(seed) {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}

function streamChunks(parser, message) {
  let result = [];
  const rng = seededRandom(42);
  const chars = Array.from(message);

  let index = 0;
  while (index < chars.length) {
    const remaining = chars.length - index;
    const chunkChars = Math.min(remaining, Math.floor(rng() * 10) + 1);
    result = parser.processChunk(chars.slice(index, index + chunkChars).join(""));
    index += chunkChars;
  }

  return result;
}

function testParser() {
  const tools = [
    ["read_file", ["path", "start_line", "end_line"]],
    ["write_to_file", ["path", "content", "line_count"]],
    ["browser_action", []],
    ["search_files", ["regex", "path"]],
    ["execute_command", ["command"]],
    ["ask_followup_question", ["question", "follow_up"]],
    ["new_rule", []],
  ];

  const toolNames = tools.map(([name]) => name);
  const toolParamNames = tools.flatMap(([, params]) => params);
  return new AssistantMessageParser(toolNames, toolParamNames);
}

function nonEmpty(block) {
  return !(block.type === "text" && block.content.length === 0);
}

const tests = [];
function test(name, fn) {
  tests.push([name, fn]);
}

test("accumulates simple text chunk by chunk", () => {
  const parser = testParser();
  const message = "Hello, this is a test.";
  const result = streamChunks(parser, message);

  assert.equal(result.length, 1);
  assert.equal(result[0].type, "text");
  assert.equal(result[0].content, message);
  assert.equal(result[0].partial, true);
});

test("accumulates multi line text chunk by chunk", () => {
  const parser = testParser();
  const message = "Line 1\nLine 2\nLine 3";
  const result = streamChunks(parser, message);

  assert.equal(result[0].type, "text");
  assert.equal(result[0].content, message);
  assert.equal(result[0].partial, true);
});

test("parses tool use with parameter streamed", () => {
  const parser = testParser();
  const result = streamChunks(
    parser,
    "<read_file><path>src/file.ts</path></read_file>",
  ).filter(nonEmpty);

  assert.equal(result.length, 1);
  assert.equal(result[0].type, "tool_use");
  assert.equal(result[0].name, "read_file");
  assert.equal(result[0].params.path, "src/file.ts");
  assert.equal(result[0].partial, false);
});

test("marks unclosed tool use as partial", () => {
  const parser = testParser();
  const result = streamChunks(parser, "<read_file><path>src/file.ts</path>").filter(nonEmpty);

  assert.equal(result[0].type, "tool_use");
  assert.equal(result[0].params.path, "src/file.ts");
  assert.equal(result[0].partial, true);
});

test("handles partial parameter in tool use", () => {
  const parser = testParser();
  const result = streamChunks(parser, "<read_file><path>src/file").filter(nonEmpty);

  assert.equal(result[0].type, "tool_use");
  assert.equal(result[0].params.path, "src/file");
  assert.equal(result[0].partial, true);
});

test("handles multiple parameters streamed", () => {
  const parser = testParser();
  const result = streamChunks(
    parser,
    "<read_file><path>src/file.ts</path><start_line>10</start_line><end_line>20</end_line></read_file>",
  ).filter(nonEmpty);

  assert.equal(result[0].type, "tool_use");
  assert.equal(result[0].params.path, "src/file.ts");
  assert.equal(result[0].params.start_line, "10");
  assert.equal(result[0].params.end_line, "20");
  assert.equal(result[0].partial, false);
});

test("parses text followed by tool use", () => {
  const parser = testParser();
  const result = streamChunks(
    parser,
    "Text before tool <read_file><path>src/file.ts</path></read_file>",
  );

  assert.equal(result.length, 2);
  assert.equal(result[0].type, "text");
  assert.equal(result[0].content, "Text before tool");
  assert.equal(result[0].partial, false);
  assert.equal(result[1].type, "tool_use");
  assert.equal(result[1].params.path, "src/file.ts");
  assert.equal(result[1].partial, false);
});

test("parses tool use followed by text", () => {
  const parser = testParser();
  const result = streamChunks(
    parser,
    "<read_file><path>src/file.ts</path></read_file>Text after tool",
  ).filter(nonEmpty);

  assert.equal(result.length, 2);
  assert.equal(result[0].type, "tool_use");
  assert.equal(result[0].params.path, "src/file.ts");
  assert.equal(result[0].partial, false);
  assert.equal(result[1].type, "text");
  assert.equal(result[1].content, "Text after tool");
  assert.equal(result[1].partial, true);
});

test("handles lost tool prefix and text chunk generator", () => {
  const parser = testParser();
  let result = streamChunks(parser, "<read_file").filter(nonEmpty);
  assert.equal(result[0].type, "text");
  assert.equal(result[0].content, "<read_file");

  parser.reset();
  let finalMessage = "";
  for (const ch of Array.from("abc<read_file><path>src/file.ts</path></read_file> Text after tool")) {
    parser.processChunk(ch);
    for (let chunk = parser.nextTextChunk(); chunk != null; chunk = parser.nextTextChunk()) {
      finalMessage += chunk;
    }
  }
  assert.equal(finalMessage, "abc Text after tool");

  parser.reset();
  finalMessage = "";
  for (const ch of Array.from("abc<read_file><p")) {
    parser.processChunk(ch);
    for (let chunk = parser.nextTextChunk(); chunk != null; chunk = parser.nextTextChunk()) {
      finalMessage += chunk;
    }
  }
  assert.equal(finalMessage, "abc");

  parser.reset();
  finalMessage = "";
  for (const ch of Array.from("abc<")) {
    parser.processChunk(ch);
    for (let chunk = parser.nextTextChunk(); chunk != null; chunk = parser.nextTextChunk()) {
      finalMessage += chunk;
    }
  }
  assert.equal(finalMessage, "abc");

  parser.reset();
  finalMessage = "";
  for (const ch of Array.from("abc<read>daf")) {
    parser.processChunk(ch);
    for (let chunk = parser.nextTextChunk(); chunk != null; chunk = parser.nextTextChunk()) {
      finalMessage += chunk;
    }
  }
  assert.equal(finalMessage, "abc<read>daf");
});

test("parses multiple tool uses separated by text", () => {
  const parser = testParser();
  const result = streamChunks(
    parser,
    "First: <read_file><path>file1.ts</path></read_file>Second: <read_file><path>file2.ts</path></read_file>",
  );

  assert.equal(result.length, 4);
  assert.equal(result[0].type, "text");
  assert.equal(result[0].content, "First:");
  assert.equal(result[1].type, "tool_use");
  assert.equal(result[1].params.path, "file1.ts");
  assert.equal(result[2].type, "text");
  assert.equal(result[2].content, "Second:");
  assert.equal(result[3].type, "tool_use");
  assert.equal(result[3].params.path, "file2.ts");
});

test("handles write_to_file content containing closing tags", () => {
  const parser = testParser();
  const message = `<write_to_file><path>src/file.ts</path><content>
function example() {
// This has XML-like content: </content>
return true;
}
</content><line_count>5</line_count></write_to_file>`;
  const result = streamChunks(parser, message).filter(nonEmpty);

  assert.equal(result[0].type, "tool_use");
  assert.equal(result[0].params.path, "src/file.ts");
  assert.equal(result[0].params.line_count, "5");
  assert.match(result[0].params.content, /function example\(\)/);
  assert.match(result[0].params.content, /XML-like content: <\/content>/);
  assert.match(result[0].params.content, /return true;/);
  assert.equal(result[0].partial, false);
});

test("handles empty messages", () => {
  const parser = testParser();
  assert.deepEqual(streamChunks(parser, ""), []);
});

test("treats malformed tool use tags as plain text", () => {
  const parser = testParser();
  const message = "This has a <not_a_tool>malformed tag</not_a_tool>";
  const result = streamChunks(parser, message);

  assert.equal(result.length, 1);
  assert.equal(result[0].type, "text");
  assert.equal(result[0].content, message);
});

test("handles tool use with no parameters", () => {
  const parser = testParser();
  const result = streamChunks(parser, "<browser_action></browser_action>").filter(nonEmpty);

  assert.equal(result[0].type, "tool_use");
  assert.equal(result[0].name, "browser_action");
  assert.deepEqual(result[0].params, {});
  assert.equal(result[0].partial, false);
});

test("handles xml like parameter content", () => {
  const parser = testParser();
  const result = streamChunks(
    parser,
    "<search_files><regex><div>.*</div></regex><path>src</path></search_files>",
  ).filter(nonEmpty);

  assert.equal(result[0].type, "tool_use");
  assert.equal(result[0].params.regex, "<div>.*</div>");
  assert.equal(result[0].params.path, "src");
  assert.equal(result[0].partial, false);
});

test("handles consecutive tool uses", () => {
  const parser = testParser();
  const result = streamChunks(
    parser,
    "<read_file><path>file1.ts</path></read_file><read_file><path>file2.ts</path></read_file>",
  ).filter(nonEmpty);

  assert.equal(result.length, 2);
  assert.equal(result[0].params.path, "file1.ts");
  assert.equal(result[1].params.path, "file2.ts");
});

test("trims non content parameters", () => {
  const parser = testParser();
  const result = streamChunks(
    parser,
    "<read_file><path>  src/file.ts  </path></read_file>",
  ).filter(nonEmpty);

  assert.equal(result[0].type, "tool_use");
  assert.equal(result[0].params.path, "src/file.ts");
});

test("keeps multi line content parameter", () => {
  const parser = testParser();
  const message = `<write_to_file><path>file.ts</path><content>
line 1
line 2
line 3
</content><line_count>3</line_count></write_to_file>`;
  const result = streamChunks(parser, message).filter(nonEmpty);

  assert.equal(result[0].type, "tool_use");
  assert.match(result[0].params.content, /line 1/);
  assert.match(result[0].params.content, /line 2/);
  assert.match(result[0].params.content, /line 3/);
  assert.equal(result[0].params.line_count, "3");
});

test("handles complex message with multiple content types", () => {
  const parser = testParser();
  const message = `I'll help you with that task.

<read_file><path>src/index.ts</path></read_file>

Now let's modify the file:

<write_to_file><path>src/index.ts</path><content>
// Updated content
console.log("Hello world");
</content><line_count>2</line_count></write_to_file>

Let's run the code:

<execute_command><command>node src/index.ts</command></execute_command>`;
  const result = streamChunks(parser, message);

  assert.equal(result.length, 6);
  assert.equal(result[0].type, "text");
  assert.equal(result[0].content, "I'll help you with that task.");
  assert.equal(result[1].name, "read_file");
  assert.match(result[2].content, /Now let's modify the file:/);
  assert.equal(result[3].name, "write_to_file");
  assert.match(result[4].content, /Let's run the code:/);
  assert.equal(result[5].name, "execute_command");
});

test("errors when max accumulator size exceeded", () => {
  const parser = testParser();
  const largeMessage = "x".repeat(1024 * 1024 + 1);

  assert.throws(
    () => parser.processChunk(largeMessage),
    /Assistant message exceeds maximum allowed size/,
  );
});

test("gracefully handles parameter exceeding max length", () => {
  const parser = testParser();
  const largeParamValue = "x".repeat(1024 * 100 + 1);

  parser.processChunk("<write_to_file><path>test.txt</path><content>");
  for (let i = 0; i < largeParamValue.length; i += 1000) {
    parser.processChunk(largeParamValue.slice(i, i + 1000));
  }
  const result = parser.processChunk("</content></write_to_file>After tool");

  const tool = result.find((block) => block.type === "tool_use");
  assert.equal(tool.params.path, "test.txt");
  assert.ok(result.some((block) => block.type === "text" && block.content.includes("After tool")));
});

test("finalizes content blocks", () => {
  const parser = testParser();
  streamChunks(parser, "<read_file><path>src/file.ts");
  parser.finalizeContentBlocks();

  assert.ok(parser.getContentBlocks().every((block) => !block.partial));
});

test("handles ask followup question", () => {
  const parser = testParser();
  const message = `Example: Requesting to ask the user for the path to the frontend-config.json file
<ask_followup_question>
<question>What is the path to the frontend-config.json file?</question>
<follow_up>
<suggest>./src/frontend-config.json</suggest>
<suggest>./config/frontend-config.json</suggest>
<suggest>./frontend-config.json</suggest>
</follow_up>
</ask_followup_question>`;
  streamChunks(parser, message);
  parser.finalizeContentBlocks();

  assert.ok(parser.getContentBlocks().every((block) => !block.partial));
});

test("thinking tag is text", () => {
  const parser = testParser();
  const result = streamChunks(
    parser,
    "<thinking>I'm thinking...</thinking> hello world",
  ).filter(nonEmpty);

  assert.equal(result.length, 1);
  assert.equal(result[0].type, "text");
});

test("search files block has expected shape", () => {
  const parser = testParser();
  streamChunks(
    parser,
    "<search_files><regex><div>.*</div></regex><path>src</path></search_files>",
  );
  parser.finalizeContentBlocks();
  const blocks = parser.getContentBlocks();
  const tool = blocks.at(-1);

  assert.ok(blocks.every((block) => !block.partial));
  assert.equal(tool.type, "tool_use");
  assert.equal(tool.name, "search_files");
  assert.equal(tool.params.regex, "<div>.*</div>");
  assert.equal(tool.params.path, "src");
});

test("streams display text while collecting only completed tool calls", () => {
  const parser = testParser();
  const message = [
    "I will inspect the file. ",
    "<read_fil",
    "e><path>src/mai",
    "n.rs</path></read_file>",
    " Then I will search. ",
    "<search_files><regex>fn main</regex><path>src</path></search_files>",
    " Done.",
  ].join("");

  let showText = "";
  const completedToolXml = [];
  let seenCompletedToolCount = 0;

  for (const ch of Array.from(message)) {
    const blocks = parser.processChunk(ch);

    for (let chunk = parser.nextTextChunk(); chunk != null; chunk = parser.nextTextChunk()) {
      showText += chunk;
    }

    const completedTools = blocks.filter(
      (block) => block.type === "tool_use" && !block.partial,
    );

    for (const tool of completedTools.slice(seenCompletedToolCount)) {
      completedToolXml.push(tool.xml);
    }
    seenCompletedToolCount = completedTools.length;
  }

  assert.equal(showText, "I will inspect the file. Then I will search. Done.");
  assert.equal(completedToolXml.length, 2);
  assert.equal(completedToolXml[0], "<read_file>\n<path>src/main.rs</path>\n</read_file>");
  assert.equal(
    completedToolXml[1],
    "<search_files>\n<regex>fn main</regex>\n<path>src</path>\n</search_files>",
  );
});

let passed = 0;
for (const [name, fn] of tests) {
  try {
    fn();
    passed += 1;
  } catch (error) {
    console.error(`not ok - ${name}`);
    throw error;
  }
}

console.log(`ok - ${passed} wasm/node tests passed`);
