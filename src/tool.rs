use rmcp::handler::server::tool::schema_for_type;
use rmcp::model::{Content, PromptArgument, PromptMessage};
use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

// Import log for schema validation logging
use log;

use crate::error::McpError;

// ============================================================================
// PERFORMANCE OPTIMIZATIONS
// ============================================================================

/// Type alias for the schema cache to reduce complexity
type SchemaCache =
    parking_lot::RwLock<HashMap<&'static str, std::sync::Arc<serde_json::Map<String, Value>>>>;

/// Schema cache to avoid repeated serialization
static SCHEMA_CACHE: std::sync::LazyLock<SchemaCache> =
    std::sync::LazyLock::new(|| parking_lot::RwLock::new(HashMap::new()));

// ============================================================================
// CORE TRAIT
// ============================================================================

/// Core trait that all tools must implement
///
/// Tools are STRUCTS that hold their own dependencies (`GitClient`, `GitHubClient`, etc.)
/// The trait is generic and knows nothing about specific services.
/// Every method (except execute/prompt) has a sensible default.
pub trait Tool: Send + Sync + Sized + 'static {
    /// Tool execution arguments (auto-generates schema via `JsonSchema`)
    type Args: DeserializeOwned + Serialize + JsonSchema + Send + 'static;

    /// Prompt arguments (what context does teaching need?)
    type PromptArgs: DeserializeOwned + JsonSchema + Send + 'static;

    // ========================================================================
    // IDENTITY (Required)
    // ========================================================================

    /// Unique tool name (e.g., "`git_clone`")
    fn name() -> &'static str;

    /// Human-readable description of what this tool does
    fn description() -> &'static str;

    // ========================================================================
    // SCHEMA (Auto-generated with caching)
    // ========================================================================

    /// Validate that schema generation works for this tool's Args type.
    /// 
    /// This method attempts to generate the JSON schema and catches any panics
    /// that occur during schema generation. Should be called during tool 
    /// registration to catch schema issues early.
    ///
    /// # Returns
    /// - `Ok(())` if schema generation succeeds
    /// - `Err(String)` with detailed error message if schema generation fails
    fn validate_schema() -> Result<(), String> {
        // Wrap schema_for_type in a panic catch
        let result = std::panic::catch_unwind(|| {
            let _ = schema_for_type::<Self::Args>();
        });
        
        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                let error_msg = if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic during schema generation".to_string()
                };
                Err(format!(
                    "Schema generation failed for tool '{}': {}",
                    Self::name(),
                    error_msg
                ))
            }
        }
    }

    /// Input schema - AUTO-GENERATED from Args type via `JsonSchema` derive
    /// Cached for performance - schema is computed once and reused
    #[inline]
    fn input_schema() -> std::sync::Arc<serde_json::Map<String, Value>> {
        let name = Self::name();

        // Fast path: read from cache
        if let Some(schema) = SCHEMA_CACHE.read().get(name) {
            return schema.clone();
        }

        // Slow path: generate and cache
        // Log schema generation attempt
        log::debug!("Generating schema for tool: {}", name);
        
        // Validate schema generation with panic catching
        if let Err(e) = Self::validate_schema() {
            log::error!("{}", e);
            // For now, still proceed but with warning - could be made fatal in future
            log::warn!("Tool '{}' registered with potentially invalid schema", name);
        } else {
            log::info!("✓ Schema generated successfully for tool: {}", name);
        }
        
        let schema = std::sync::Arc::new(schema_for_type::<Self::Args>());
        SCHEMA_CACHE.write().insert(name, schema.clone());
        schema
    }

    /// Output schema (optional, rarely needed)
    #[must_use]
    #[inline]
    fn output_schema() -> Option<std::sync::Arc<serde_json::Map<String, Value>>> {
        None
    }

    // ========================================================================
    // BEHAVIOR ANNOTATIONS (Tool IS its behavior)
    // ========================================================================

    /// Does this tool only read (never modify) state?
    ///
    /// true = read-only (safe, can't break things)
    /// false = writes/modifies state (requires caution)
    ///
    /// Default: true (assumes read-only by default - safe default)
    #[must_use]
    #[inline]
    fn read_only() -> bool {
        true
    }

    /// Can this tool delete or overwrite existing data?
    ///
    /// Only meaningful when `read_only` = false.
    /// true = can delete/overwrite (dangerous)
    /// false = only adds/creates (safer)
    ///
    /// Default: false (assumes non-destructive by default - safe default)
    #[must_use]
    #[inline]
    fn destructive() -> bool {
        false
    }

    /// Is calling this tool repeatedly with same args safe/idempotent?
    ///
    /// Only meaningful when `read_only` = false.
    /// true = safe to retry (same result every time)
    /// false = each call has different effect
    ///
    /// Default: true (assumes idempotent by default - safe default)
    #[must_use]
    #[inline]
    fn idempotent() -> bool {
        true
    }

    /// Does this tool interact with external systems (network, filesystem outside repo)?
    ///
    /// true = open world (network calls, external APIs, can fail due to external factors)
    /// false = closed world (only local operations, deterministic)
    ///
    /// Default: false (assumes local operations by default - safe default)
    #[must_use]
    #[inline]
    fn open_world() -> bool {
        false
    }

    // ========================================================================
    // EXECUTION (Required)
    // ========================================================================

    /// Execute the tool with given arguments
    ///
    /// This is where the actual work happens.
    /// Since tools are structs holding their dependencies, we use &self.
    fn execute(
        &self,
        args: Self::Args,
        ctx: ToolExecutionContext,
    ) -> impl std::future::Future<Output = Result<Vec<Content>, McpError>> + Send;

    // ========================================================================
    // PROMPTING (Required - agents need this!)
    // ========================================================================

    /// Prompt name (defaults to "{`tool_name`}_help")
    #[must_use]
    #[inline]
    fn prompt_name() -> Cow<'static, str> {
        Cow::Owned(format!("{}_help", Self::name()))
    }

    /// Prompt description (defaults to tool description)
    #[must_use]
    #[inline]
    fn prompt_description() -> &'static str {
        Self::description()
    }

    /// What arguments does the teaching prompt accept?
    ///
    /// These let agents customize what they want to learn about.
    /// Example: "repo" (which repo?), "shallow" (learn about shallow clones?)
    fn prompt_arguments() -> Vec<PromptArgument>;

    /// Generate teaching conversation for this tool
    ///
    /// Returns a conversation showing agent how/when to use the tool.
    /// Should include examples, common patterns, gotchas, requirements.
    /// Since tools are structs, we use &self here too.
    fn prompt(
        &self,
        args: Self::PromptArgs,
    ) -> impl std::future::Future<Output = Result<Vec<PromptMessage>, McpError>> + Send;

    // ========================================================================
    // RMCP INTEGRATION (Default implementations)
    // ========================================================================

    /// Convert this tool into an RMCP `ToolRoute`
    ///
    /// This default implementation builds the route from trait methods.
    /// Tools get this for free - no need to implement `IntoToolRoute` manually.
    fn into_tool_route<S>(self) -> rmcp::handler::server::router::tool::ToolRoute<S>
    where
        S: Send + Sync + 'static,
    {
        use rmcp::handler::server::router::tool::ToolRoute;
        use rmcp::model::{Tool as RmcpTool, ToolAnnotations};
        use std::sync::Arc;

        // Build annotations from trait methods
        let annotations = ToolAnnotations::new()
            .read_only(Self::read_only())
            .destructive(Self::destructive())
            .idempotent(Self::idempotent())
            .open_world(Self::open_world());

        // Build RMCP Tool metadata
        let metadata = RmcpTool {
            name: Self::name().into(),
            title: None,
            description: Some(Self::description().into()),
            input_schema: Self::input_schema(),
            output_schema: Self::output_schema(),
            annotations: Some(annotations),
            icons: None,
            meta: None,
        };

        // Create handler with ToolHandler wrapper (HRTB-compatible, zero-cost)
        let handler = ToolHandler {
            tool: Arc::new(self),
        };

        // Use ToolRoute::new() - handles HRTB internally
        ToolRoute::new(metadata, handler)
    }

    /// Convert this tool into an RMCP `PromptRoute`
    ///
    /// This default implementation builds the route from trait methods.
    /// Tools get this for free - no need to implement `IntoPromptRoute` manually.
    fn into_prompt_route<S>(self) -> rmcp::handler::server::router::prompt::PromptRoute<S>
    where
        S: Send + Sync + 'static,
    {
        use rmcp::handler::server::router::prompt::PromptRoute;
        use rmcp::handler::server::wrapper::Parameters;
        use rmcp::model::{GetPromptResult, Prompt as RmcpPrompt};
        use std::sync::Arc;

        // Build RMCP Prompt metadata
        let metadata = RmcpPrompt {
            name: Self::prompt_name().into_owned(),
            title: None,
            description: Some(Self::prompt_description().to_string()),
            arguments: Some(Self::prompt_arguments()),
            icons: None,
        };

        // Wrap self in Arc for handler
        let tool = Arc::new(self);

        // Handler captures the tool instance
        let handler = move |Parameters(args): Parameters<Self::PromptArgs>| {
            let tool = tool.clone();
            async move {
                let messages = tool
                    .prompt(args)
                    .await
                    .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?;

                Ok(GetPromptResult {
                    description: Some(Self::prompt_description().to_string()),
                    messages,
                })
            }
        };

        PromptRoute::new(metadata, handler)
    }

    /// Convert Arc-wrapped tool into an RMCP `ToolRoute` (optimized - no extra Arc allocation)
    ///
    /// This is more efficient than `into_tool_route(self)` when the tool is already wrapped in Arc.
    /// The tool is used directly without creating an additional Arc wrapper.
    fn arc_into_tool_route<S>(self: Arc<Self>) -> rmcp::handler::server::router::tool::ToolRoute<S>
    where
        S: Send + Sync + 'static,
    {
        use rmcp::handler::server::router::tool::ToolRoute;
        use rmcp::model::{Tool as RmcpTool, ToolAnnotations};

        // Build annotations from trait methods
        let annotations = ToolAnnotations::new()
            .read_only(Self::read_only())
            .destructive(Self::destructive())
            .idempotent(Self::idempotent())
            .open_world(Self::open_world());

        // Build RMCP Tool metadata
        let metadata = RmcpTool {
            name: Self::name().into(),
            title: None,
            description: Some(Self::description().into()),
            input_schema: Self::input_schema(),
            output_schema: Self::output_schema(),
            annotations: Some(annotations),
            icons: None,
            meta: None,
        };

        // Create handler with ToolHandler wrapper (HRTB-compatible, zero-cost)
        // Use self directly (already Arc<Self>) - no extra Arc allocation
        let handler = ToolHandler {
            tool: self,
        };

        // Use ToolRoute::new() - handles HRTB internally
        ToolRoute::new(metadata, handler)
    }

    /// Convert Arc-wrapped tool into an RMCP `PromptRoute` (optimized - no extra Arc allocation)
    ///
    /// This is more efficient than `into_prompt_route(self)` when the tool is already wrapped in Arc.
    /// The tool is used directly without creating an additional Arc wrapper.
    fn arc_into_prompt_route<S>(
        self: Arc<Self>,
    ) -> rmcp::handler::server::router::prompt::PromptRoute<S>
    where
        S: Send + Sync + 'static,
    {
        use rmcp::handler::server::router::prompt::PromptRoute;
        use rmcp::handler::server::wrapper::Parameters;
        use rmcp::model::{GetPromptResult, Prompt as RmcpPrompt};

        // Build RMCP Prompt metadata
        let metadata = RmcpPrompt {
            name: Self::prompt_name().into_owned(),
            title: None,
            description: Some(Self::prompt_description().to_string()),
            arguments: Some(Self::prompt_arguments()),
            icons: None,
        };

        // Use self directly (already Arc<Self>)
        let tool = self;

        // Handler captures the Arc<Tool>
        let handler = move |Parameters(args): Parameters<Self::PromptArgs>| {
            let tool = tool.clone(); // Cheap Arc clone
            async move {
                let messages = tool
                    .prompt(args)
                    .await
                    .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?;

                Ok(GetPromptResult {
                    description: Some(Self::prompt_description().to_string()),
                    messages,
                })
            }
        };

        PromptRoute::new(metadata, handler)
    }
}

// ============================================================================
// PROGRESS NOTIFICATION CONTEXT
// ============================================================================

/// Execution context provided to tools for progress notifications and cancellation.
///
/// Supports three patterns:
/// 1. Stream text messages: `ctx.stream("output\n")`
/// 2. Report numeric progress: `ctx.progress(50, 100)`  
/// 3. Combined: `ctx.update(50, 100, "Processing file 50/100")`
#[derive(Clone)]
pub struct ToolExecutionContext {
    /// Peer interface for sending progress notifications
    peer: rmcp::service::Peer<rmcp::RoleServer>,
    
    /// Cancellation token (tool should check periodically)
    ct: tokio_util::sync::CancellationToken,
    
    /// Unique request identifier (used for progress_token)
    request_id: rmcp::model::RequestId,
}

impl ToolExecutionContext {
    /// Create a new ToolExecutionContext with the given peer, cancellation token, and request ID.
    ///
    /// This constructor is public to allow custom integration contexts (e.g., bridging
    /// to non-RMCP transports or in-process sessions). Most tools should not need to
    /// call this directly - the context is typically provided by the RMCP framework.
    ///
    /// # Arguments
    /// * `peer` - RMCP peer for sending progress notifications
    /// * `ct` - Cancellation token for this execution
    /// * `request_id` - Unique identifier for this request
    #[must_use]
    pub fn new(
        peer: rmcp::service::Peer<rmcp::RoleServer>,
        ct: tokio_util::sync::CancellationToken,
        request_id: rmcp::model::RequestId,
    ) -> Self {
        Self {
            peer,
            ct,
            request_id,
        }
    }

    /// Stream a text message (for terminal output, logs, status updates).
    ///
    /// Use this for incrementally streaming text output as it becomes available.
    ///
    /// # Example
    /// ```
    /// // Terminal streaming command output
    /// ctx.stream("npm info using npm@8.19.2\n").await.ok();
    /// ctx.stream("added 234 packages in 15s\n").await.ok();
    /// ```
    pub async fn stream(&self, message: impl Into<String>) -> Result<(), McpError> {
        self.notify_internal(0.0, None, Some(message.into())).await
    }

    /// Report numeric progress (for progress bars, counters).
    ///
    /// Use this when you have a known total and current progress value.
    ///
    /// # Example
    /// ```
    /// // Processing 50 out of 100 files
    /// ctx.progress(50.0, 100.0).await.ok();
    /// ```
    pub async fn progress(&self, current: f64, total: f64) -> Result<(), McpError> {
        self.notify_internal(current, Some(total), None).await
    }

    /// Report both numeric progress and a descriptive message.
    ///
    /// Use this when you want both a progress bar AND status text.
    ///
    /// # Example
    /// ```
    /// ctx.update(50.0, 100.0, "Generating embeddings 50/100").await.ok();
    /// ```
    pub async fn update(
        &self,
        current: f64,
        total: f64,
        message: impl Into<String>
    ) -> Result<(), McpError> {
        self.notify_internal(current, Some(total), Some(message.into())).await
    }

    /// Advanced: Full control over progress notification fields.
    ///
    /// Use this when you need fine-grained control (e.g., unknown total).
    ///
    /// # Example
    /// ```
    /// // Unknown total, just report current count
    /// ctx.notify(lines_read as f64, None, Some("Reading file...")).await.ok();
    /// ```
    pub async fn notify(
        &self,
        progress: f64,
        total: Option<f64>,
        message: Option<String>
    ) -> Result<(), McpError> {
        self.notify_internal(progress, total, message).await
    }

    /// Internal implementation - sends the actual notification
    async fn notify_internal(
        &self,
        progress: f64,
        total: Option<f64>,
        message: Option<String>
    ) -> Result<(), McpError> {
        use rmcp::model::{ProgressNotificationParam, ProgressToken, NumberOrString};

        // Generate unique progress token from request ID
        let progress_token = ProgressToken(NumberOrString::String(
            format!("tool_{}", match &self.request_id {
                NumberOrString::Number(n) => n.to_string(),
                NumberOrString::String(s) => s.to_string(),
            }).into()
        ));

        let params = ProgressNotificationParam {
            progress_token,
            progress,
            total,
            message,
        };

        self.peer
            .notify_progress(params)
            .await
            .map_err(|e| McpError::Other(anyhow::anyhow!(
                "Failed to send progress notification: {}", e
            )))
    }

    /// Check if tool execution was cancelled by the client.
    ///
    /// Tools should check this periodically during long operations.
    ///
    /// # Example
    /// ```
    /// for item in items {
    ///     if ctx.is_cancelled() {
    ///         return Err(McpError::cancelled("Operation cancelled"));
    ///     }
    ///     process_item(item).await?;
    /// }
    /// ```
    pub fn is_cancelled(&self) -> bool {
        self.ct.is_cancelled()
    }

    /// Get the cancellation token for use with `tokio::select!` or custom logic.
    pub fn cancellation_token(&self) -> &tokio_util::sync::CancellationToken {
        &self.ct
    }
}

// ============================================================================
// FromContextPart implementation for ToolExecutionContext
// ============================================================================

impl<S> rmcp::handler::server::common::FromContextPart<rmcp::handler::server::tool::ToolCallContext<'_, S>> 
    for ToolExecutionContext
where
    S: Send + Sync + 'static,
{
    fn from_context_part(
        context: &mut rmcp::handler::server::tool::ToolCallContext<'_, S>
    ) -> Result<Self, rmcp::ErrorData> {
        Ok(ToolExecutionContext {
            peer: context.request_context.peer.clone(),
            ct: context.request_context.ct.clone(),
            request_id: context.request_context.id.clone(),
        })
    }
}

// ============================================================================
// ToolHandler - Zero-cost wrapper for CallToolHandler implementation
// ============================================================================

/// Wrapper struct that holds a tool and implements CallToolHandler.
/// This enables HRTB-compatible tool routing without closure lifetime issues.
struct ToolHandler<T: Tool> {
    tool: Arc<T>,
}

impl<T: Tool> Clone for ToolHandler<T> {
    fn clone(&self) -> Self {
        Self {
            tool: self.tool.clone(),
        }
    }
}

impl<T, S> rmcp::handler::server::tool::CallToolHandler<S, ()> for ToolHandler<T>
where
    T: Tool,
    S: Send + Sync + 'static,
{
    fn call(
        self,
        mut context: rmcp::handler::server::tool::ToolCallContext<'_, S>,
    ) -> futures::future::BoxFuture<'_, Result<rmcp::model::CallToolResult, rmcp::ErrorData>> {
        use rmcp::handler::server::wrapper::Parameters;
        use rmcp::handler::server::common::FromContextPart;
        use rmcp::model::CallToolResult;
        
        Box::pin(async move {
            let start = std::time::Instant::now();

            // Extract arguments and execution context
            let Parameters(args) = Parameters::<T::Args>::from_context_part(&mut context)?;
            let exec_ctx = ToolExecutionContext::from_context_part(&mut context)?;

            // Serialize args for history
            let args_json = serde_json::to_value(&args)
                .unwrap_or_else(|_| serde_json::json!({}));

            // Execute tool
            let result = self.tool.execute(args, exec_ctx).await;
            let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

            // Convert result to JSON for history
            let output_json = match &result {
                Ok(contents) => serde_json::json!({
                    "content_blocks": contents.len(),
                    "preview": contents.first().map(|c| format!("{:?}", c))
                }),
                Err(e) => serde_json::json!({
                    "error": e.to_string(),
                    "is_error": true
                }),
            };

            // Record to history
            if let Some(history) = crate::tool_history::get_global_history() {
                history.add_call(
                    T::name().to_string(),
                    args_json,
                    output_json,
                    Some(duration_ms),
                );
            }

            // Return formatted content
            let contents = result.map_err(rmcp::ErrorData::from)?;
            Ok(CallToolResult::success(contents))
        })
    }
}
