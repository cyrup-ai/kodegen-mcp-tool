<div align="center">
  <img src="assets/img/banner.png" alt="Kodegen AI Banner" width="100%" />
</div>

# kodegen_mcp_tool

> Memory-efficient, blazing-fast MCP tools for code generation agents.

A Rust library that provides a simple yet powerful `Tool` trait for building [Model Context Protocol (MCP)](https://modelcontextprotocol.io) tools that integrate seamlessly with the [RMCP](https://github.com/modelcontextprotocol/rust-sdk) framework.

## Features

- **Simple API** - Implement one trait, get RMCP integration, schema generation, and history tracking automatically
- **Automatic JSON Schema** - Derives from `JsonSchema` with global caching for zero overhead
- **Built-in Tool History** - Fire-and-forget recording with persistent JSONL storage, never blocks execution
- **Behavior Annotations** - Declare tool semantics (`read_only`, `destructive`, `idempotent`, `open_world`)
- **Prompting System** - Teach agents how to use your tools with conversation-style prompts
- **Performance Optimized** - Schema caching, Arc optimizations, background I/O for maximum throughput
- **Comprehensive Errors** - `McpError` enum with automatic conversion to RMCP error types
- **Production Ready** - Thread-safe, async-first, with atomic file operations and rotation

## Requirements

- **Rust nightly** - This library uses Rust edition 2024
- **Tokio runtime** - Async execution powered by Tokio

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
kodegen_mcp_tool = "0.1.0"
```

Or install from git:

```toml
[dependencies]
kodegen_mcp_tool = { git = "https://github.com/cyrup-ai/kodegen-mcp-tool" }
```

## Quick Start

Here's a minimal example of implementing a tool:

```rust
use kodegen_mcp_tool::{Tool, error::McpError};
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use serde_json::Value;
use rmcp::model::{PromptArgument, PromptMessage};

// 1. Define your tool struct (holds dependencies)
pub struct EchoTool;

// 2. Define input arguments
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct EchoArgs {
    message: String,
}

// 3. Define prompt arguments
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct EchoPromptArgs {}

// 4. Implement the Tool trait
impl Tool for EchoTool {
    type Args = EchoArgs;
    type PromptArgs = EchoPromptArgs;

    fn name() -> &'static str { "echo" }
    fn description() -> &'static str { "Echoes back your message" }

    async fn execute(&self, args: Self::Args) -> Result<Value, McpError> {
        Ok(serde_json::json!({
            "echo": args.message
        }))
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![])
    }
}

// 5. Register with RMCP router
use rmcp::handler::server::router::RouterService;

#[tokio::main]
async fn main() {
    // Initialize tool history
    kodegen_mcp_tool::tool_history::init_global_history("my-server".to_string()).await;

    // Create router and register tool
    let router = RouterService::new("echo-server")
        .tool(EchoTool.into_tool_route())
        .prompt(EchoTool.into_prompt_route());

    // Start server (example with stdio transport)
    // ... your transport setup here ...
}
```

## Behavior Annotations

Tools can declare their behavior by overriding trait methods:

```rust
impl Tool for MyTool {
    // ... other trait methods ...

    fn read_only() -> bool { false }      // Tool modifies state
    fn destructive() -> bool { true }     // Can delete/overwrite data
    fn idempotent() -> bool { false }     // Each call has different effect
    fn open_world() -> bool { true }      // Interacts with external systems
}
```

These annotations are automatically converted to RMCP `ToolAnnotations` and help agents understand tool safety characteristics.

## Tool History

Tool call history is automatically tracked for all tools:

```rust
// Initialize once at startup
kodegen_mcp_tool::tool_history::init_global_history("server-id".to_string()).await;

// Access anywhere in your code
if let Some(history) = kodegen_mcp_tool::tool_history::get_global_history() {
    let recent_calls = history.get_recent_calls(
        100,                    // max results
        0,                      // offset (negative = tail)
        Some("my_tool"),        // filter by tool name
        None,                   // filter by timestamp
    ).await;
}
```

Features:
- Fire-and-forget recording (never blocks tool execution)
- In-memory cache (last 1000 entries)
- Persistent JSONL storage
- Automatic file rotation at 5000 entries
- Background disk I/O (1 second flush interval)

## Documentation

Generate and view the API documentation:

```bash
cargo doc --open
```

For more detailed guidance on working with this codebase, see [CLAUDE.md](CLAUDE.md).

## Architecture Highlights

- **Struct-based Tools** - Tools are stateful structs that hold their dependencies, not singletons or static functions
- **Schema Caching** - JSON schemas are computed once per tool type and cached globally using `LazyLock`
- **Arc Optimization** - `arc_into_tool_route()` variants avoid double-Arc allocation for pre-wrapped tools
- **Background Processing** - Tool history uses dedicated background task for all disk I/O

## License

Dual-licensed under either:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE.md) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE.md) or http://opensource.org/licenses/MIT)

at your option.

## Links

- **Homepage**: [https://kodegen.ai](https://kodegen.ai)
- **Repository**: [https://github.com/cyrup-ai/kodegen-mcp-tool](https://github.com/cyrup-ai/kodegen-mcp-tool)
- **Model Context Protocol**: [https://modelcontextprotocol.io](https://modelcontextprotocol.io)
- **RMCP SDK**: [https://github.com/modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk)

---

**KODEGEN.ᴀɪ** - Built by [David Maple](https://kodegen.ai)
