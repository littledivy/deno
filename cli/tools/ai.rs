// Copyright 2018-2025 the Deno authors. MIT license.

use std::env;
use std::fs;
use std::io::Write;
use std::io::{self};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use deno_ast::ModuleSpecifier;
use deno_core::error::AnyError;
use deno_core::serde_json;
use deno_runtime::WorkerExecutionMode;
use deno_runtime::deno_io::Stdio;
use deno_runtime::deno_permissions::PermissionsContainer;
use dissimilar::Chunk;
use dissimilar::diff;
use percent_encoding::NON_ALPHANUMERIC;
use percent_encoding::utf8_percent_encode;
use reqwest::Client;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use serde::Deserialize;
use serde::Serialize;

use crate::args::AiFlags;
use crate::args::Flags;
use crate::factory::CliFactory;
use crate::worker::CliMainWorkerFactory;

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicRequest {
  model: String,
  max_tokens: u32,
  messages: Vec<AnthropicMessage>,
  tools: Option<Vec<Tool>>,
  stream: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AnthropicMessage {
  role: String,
  content: MessageContent,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum MessageContent {
  Text(String),
  Array(Vec<ContentBlock>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ContentBlock {
  #[serde(rename = "type")]
  block_type: String,
  text: Option<String>,
  tool_use_id: Option<String>,
  name: Option<String>,
  input: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Tool {
  name: String,
  description: String,
  input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct CustomTool {
  name: String,
  desc: String,
  input_schema: Option<serde_json::Value>,
  // We don't need to deserialize the fn, we'll call it via deno runtime
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
  content: Vec<ContentBlock>,
  stop_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIRequest {
  model: String,
  messages: Vec<OpenAIMessage>,
  tools: Option<Vec<OpenAITool>>,
  stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIMessage {
  role: String,
  content: Option<String>,
  tool_calls: Option<Vec<ToolCall>>,
  tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAITool {
  #[serde(rename = "type")]
  tool_type: String,
  function: Function,
}

#[derive(Debug, Serialize, Deserialize)]
struct Function {
  name: String,
  description: String,
  parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolCall {
  id: String,
  #[serde(rename = "type")]
  call_type: String,
  function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct FunctionCall {
  name: String,
  arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
  choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
  message: OpenAIMessage,
}

struct AiSession {
  client: Client,
  model_provider: String,
  model_name: String,
  api_key: String,
  conversation: Vec<AnthropicMessage>,
  cwd: String,
  custom_tools_config: Option<PathBuf>,
  custom_tools: Vec<CustomTool>,
  worker_factory: Arc<CliMainWorkerFactory>,
  cli_factory: Arc<CliFactory>,
}

struct LoadingIndicator {
  message: String,
  frames: Vec<&'static str>,
  frame_index: Arc<AtomicUsize>,
  is_running: Arc<AtomicBool>,
  handle: Option<thread::JoinHandle<()>>,
}

impl LoadingIndicator {
  fn new(message: String) -> Self {
    Self {
      message,
      frames: vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
      frame_index: Arc::new(AtomicUsize::new(0)),
      is_running: Arc::new(AtomicBool::new(false)),
      handle: None,
    }
  }

  fn start(&mut self) {
    self.is_running.store(true, Ordering::SeqCst);
    let is_running = Arc::clone(&self.is_running);
    let frame_index = Arc::clone(&self.frame_index);
    let message = self.message.clone();
    let frames = self.frames.clone();

    let handle = thread::spawn(move || {
      while is_running.load(Ordering::SeqCst) {
        let current_frame = frame_index.load(Ordering::SeqCst);
        print!("\r{} {}... ", frames[current_frame], message);
        io::stdout().flush().ok();
        frame_index.store((current_frame + 1) % frames.len(), Ordering::SeqCst);
        thread::sleep(Duration::from_millis(120));
      }
    });

    self.handle = Some(handle);
  }

  fn stop(&mut self, result_message: Option<&str>) {
    self.is_running.store(false, Ordering::SeqCst);
    if let Some(handle) = self.handle.take() {
      handle.join().ok();
    }

    print!("\r\x1b[K"); // Clear the line
    if let Some(msg) = result_message {
      print!("✓ {} - {}\n", self.message, msg);
    } else {
      print!("✓ {}\n", self.message);
    }
    io::stdout().flush().ok();
  }

  fn error(&mut self, error_message: &str) {
    self.is_running.store(false, Ordering::SeqCst);
    if let Some(handle) = self.handle.take() {
      handle.join().ok();
    }

    print!("\r\x1b[K"); // Clear the line
    print!("✗ {} - Error: {}\n", self.message, error_message);
    io::stdout().flush().ok();
  }
}

// We'll implement this differently to avoid accessing private fields

impl AiSession {
  fn new(
    model_provider: String,
    model_name: String,
    api_key: String,
    custom_tools_config: Option<PathBuf>,
    worker_factory: Arc<CliMainWorkerFactory>,
    cli_factory: Arc<CliFactory>,
  ) -> Self {
    let cwd = env::current_dir()
      .unwrap_or_default()
      .to_string_lossy()
      .to_string();

    Self {
      client: Client::new(),
      model_provider,
      model_name,
      api_key,
      conversation: Vec::new(),
      cwd,
      custom_tools_config,
      custom_tools: Vec::new(),
      worker_factory,
      cli_factory,
    }
  }

  async fn load_custom_tools(&mut self) -> Result<(), AnyError> {
    if let Some(config_path) = &self.custom_tools_config {
      let mut loader = LoadingIndicator::new("Loading tools".to_string());
      loader.start();

      match self.load_tools_directly(config_path).await {
        Ok(tools_data) => {
          self.custom_tools = tools_data;
          loader
            .stop(Some(&format!("Loaded {} tools", self.custom_tools.len())));
        }
        Err(e) => {
          loader.error(&format!("Failed to load tools: {}", e));
          return Err(e);
        }
      }
    }
    Ok(())
  }

  // Load tools using the Deno runtime
  async fn load_tools_directly(
    &self,
    config_path: &PathBuf,
  ) -> Result<Vec<CustomTool>, AnyError> {
    let config_specifier = ModuleSpecifier::from_file_path(config_path)
      .map_err(|_| {
        AnyError::msg("Failed to convert config path to module specifier")
      })?;

    let permissions = PermissionsContainer::allow_all(
      self.cli_factory.permission_desc_parser()?.clone(),
    );

    let mut worker = self
      .worker_factory
      .create_custom_worker(
        WorkerExecutionMode::Run,
        config_specifier,
        vec![],
        permissions,
        vec![],
        Stdio::default(),
        None,
      )
      .await
      .map_err(|e| AnyError::msg(format!("Failed to create worker: {}", e)))?;

    worker.execute_main_module().await.map_err(|e| {
      AnyError::msg(format!("Failed to execute tools config: {}", e))
    })?;

    // Get the tools from globalThis.tools using execute_script_static
    let tools_script = r#"
      if (!globalThis.tools) {
        throw new Error("No globalThis.tools found. Please set globalThis.tools in your config file.");
      }
      globalThis.tools.map(tool => ({
        name: tool.name,
        desc: tool.desc,
        input_schema: tool.input_schema || null
      }))
    "#;

    let tools_value =
      worker
        .execute_script_static("get_tools", tools_script)
        .map_err(|e| AnyError::msg(format!("Failed to get tools: {}", e)))?;

    // Convert the worker to MainWorker to access js_runtime
    let mut main_worker = worker.into_main_worker();
    let runtime = &mut main_worker.js_runtime;
    let scope = &mut runtime.handle_scope();
    let tools_local = deno_core::v8::Local::new(scope, tools_value);
    let tools: Vec<CustomTool> =
      deno_core::serde_v8::from_v8(scope, tools_local).map_err(|e| {
        AnyError::msg(format!("Failed to deserialize tools: {}", e))
      })?;

    Ok(tools)
  }

  fn get_all_tools(&self) -> Vec<Tool> {
    let mut tools = vec![
      Tool {
        name: "read_file".to_string(),
        description: "Read the contents of a file".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "path": {
              "type": "string",
              "description": "The file path to read"
            }
          },
          "required": ["path"]
        }),
      },
      Tool {
        name: "write_file".to_string(),
        description: "Write content to a file".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "path": {
              "type": "string",
              "description": "The file path to write to"
            },
            "content": {
              "type": "string",
              "description": "The content to write to the file"
            }
          },
          "required": ["path", "content"]
        }),
      },
      Tool {
        name: "list_directory".to_string(),
        description: "List files and directories in a given path".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "path": {
              "type": "string",
              "description": "The directory path to list"
            }
          },
          "required": ["path"]
        }),
      },
      Tool {
        name: "execute_command".to_string(),
        description: "Execute a shell command".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "command": {
              "type": "string",
              "description": "The command to execute"
            }
          },
          "required": ["command"]
        }),
      },
      Tool {
        name: "get_docs".to_string(),
        description: "Get documentation for any TypeScript/JavaScript module using deno_doc".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "module_path": {
              "type": "string",
              "description": "The path to the module file or URL (e.g., './mod.ts', 'https://deno.land/std/fs/mod.ts')"
            },
            "filter": {
              "type": "string",
              "description": "Optional filter to show only specific symbols (e.g., 'readFile', 'MyClass')"
            }
          },
          "required": ["module_path"]
        }),
      },
      Tool {
        name: "edit_file".to_string(),
        description: "Edit a file by replacing specific content with new content, showing a diff".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "path": {
              "type": "string",
              "description": "The file path to edit"
            },
            "old_content": {
              "type": "string",
              "description": "The content to replace (must be exact match)"
            },
            "new_content": {
              "type": "string",
              "description": "The new content to replace with"
            }
          },
          "required": ["path", "old_content", "new_content"]
        }),
      },
      Tool {
        name: "jsr_search_packages".to_string(),
        description: "Search for packages on JSR registry".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "query": {
              "type": "string",
              "description": "Search query for packages"
            },
            "limit": {
              "type": "number",
              "description": "Maximum number of packages to return (1-100, default 20)"
            },
            "page": {
              "type": "number", 
              "description": "Page number for pagination (default 1)"
            }
          },
          "required": ["query"]
        }),
      },
      Tool {
        name: "jsr_get_package".to_string(),
        description: "Get detailed information about a specific JSR package".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "scope": {
              "type": "string",
              "description": "The package scope (e.g., 'std')"
            },
            "package": {
              "type": "string", 
              "description": "The package name (e.g., 'fs')"
            }
          },
          "required": ["scope", "package"]
        }),
      },
      Tool {
        name: "jsr_get_package_versions".to_string(),
        description: "Get all versions of a JSR package".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "scope": {
              "type": "string",
              "description": "The package scope (e.g., 'std')"
            },
            "package": {
              "type": "string",
              "description": "The package name (e.g., 'fs')"
            }
          },
          "required": ["scope", "package"]
        }),
      },
      Tool {
        name: "jsr_get_package_version".to_string(),
        description: "Get detailed information about a specific version of a JSR package".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "scope": {
              "type": "string",
              "description": "The package scope (e.g., 'std')"
            },
            "package": {
              "type": "string",
              "description": "The package name (e.g., 'fs')"
            },
            "version": {
              "type": "string",
              "description": "The version (e.g., '1.2.3')"
            }
          },
          "required": ["scope", "package", "version"]
        }),
      },
      Tool {
        name: "jsr_get_package_dependencies".to_string(),
        description: "Get dependencies of a specific JSR package version".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "scope": {
              "type": "string",
              "description": "The package scope (e.g., 'std')"
            },
            "package": {
              "type": "string",
              "description": "The package name (e.g., 'fs')"
            },
            "version": {
              "type": "string",
              "description": "The version (e.g., '1.2.3')"
            }
          },
          "required": ["scope", "package", "version"]
        }),
      },
    ];

    // Add custom tools
    for custom_tool in &self.custom_tools {
      let input_schema =
        custom_tool.input_schema.clone().unwrap_or_else(|| {
          serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
          })
        });

      tools.push(Tool {
        name: custom_tool.name.clone(),
        description: custom_tool.desc.clone(),
        input_schema,
      });
    }

    tools
  }

  async fn execute_tool(
    &self,
    name: &str,
    input: &serde_json::Value,
  ) -> Result<String, AnyError> {
    let mut loader = LoadingIndicator::new(format!("+ {}({})", name, input));
    loader.start();

    let result = match name {
      "read_file" => {
        let path = input["path"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing path"))?;
        let content = fs::read_to_string(path)
          .map_err(|e| AnyError::msg(format!("Failed to read file: {}", e)))?;
        Ok(content)
      }
      "write_file" => {
        let path = input["path"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing path"))?;
        let content = input["content"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing content"))?;

        if let Some(parent) = Path::new(path).parent() {
          fs::create_dir_all(parent).map_err(|e| {
            AnyError::msg(format!("Failed to create directories: {}", e))
          })?;
        }

        // Check if file exists to show diff
        let existing_content = fs::read_to_string(path).ok();

        fs::write(path, content)
          .map_err(|e| AnyError::msg(format!("Failed to write file: {}", e)))?;

        if let Some(old_content) = existing_content {
          if old_content != content {
            // Generate diff for existing file modification
            let diff_chunks = diff(&old_content, content);
            let mut diff_output = String::new();

            for chunk in &diff_chunks {
              match chunk {
                Chunk::Equal(text) => {
                  // Only show a few lines of context around changes
                  let lines: Vec<&str> = text.lines().collect();
                  if lines.len() > 6 {
                    for line in lines.iter().take(3) {
                      diff_output.push_str(&format!("  {}\n", line));
                    }
                    if lines.len() > 6 {
                      diff_output.push_str("  ...\n");
                    }
                    for line in lines.iter().skip(lines.len().saturating_sub(3))
                    {
                      diff_output.push_str(&format!("  {}\n", line));
                    }
                  } else {
                    for line in lines {
                      diff_output.push_str(&format!("  {}\n", line));
                    }
                  }
                }
                Chunk::Delete(text) => {
                  for line in text.lines() {
                    diff_output
                      .push_str(&format!("\x1b[31m- {}\x1b[0m\n", line));
                  }
                }
                Chunk::Insert(text) => {
                  for line in text.lines() {
                    diff_output
                      .push_str(&format!("\x1b[32m+ {}\x1b[0m\n", line));
                  }
                }
              }
            }

            Ok(format!(
              "Successfully updated {}\n\nDiff:\n{}\n\nFile has been updated with the changes.",
              path, diff_output
            ))
          } else {
            Ok(format!(
              "File {} already contains the same content - no changes made",
              path
            ))
          }
        } else {
          Ok(format!("Successfully created new file: {}", path))
        }
      }
      "list_directory" => {
        let path = input["path"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing path"))?;
        let entries = fs::read_dir(path)
          .map_err(|e| {
            AnyError::msg(format!("Failed to read directory: {}", e))
          })?
          .collect::<Result<Vec<_>, _>>()
          .map_err(|e| {
            AnyError::msg(format!("Failed to read directory entries: {}", e))
          })?;

        let mut result = Vec::new();
        for entry in entries {
          let name = entry.file_name().to_string_lossy().to_string();
          let file_type = if entry.file_type()?.is_dir() {
            "directory"
          } else {
            "file"
          };
          result.push(format!("{} ({})", name, file_type));
        }
        Ok(result.join("\n"))
      }
      "execute_command" => {
        let command = input["command"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing command"))?;
        let output = std::process::Command::new("sh")
          .arg("-c")
          .arg(command)
          .current_dir(&self.cwd)
          .output()
          .map_err(|e| {
            AnyError::msg(format!("Failed to execute command: {}", e))
          })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
          Ok(stdout.to_string())
        } else {
          Ok(format!(
            "Command failed with exit code {}\nstdout: {}\nstderr: {}",
            output.status.code().unwrap_or(-1),
            stdout,
            stderr
          ))
        }
      }
      "get_docs" => {
        let module_path = input["module_path"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing module_path"))?;
        let filter = input["filter"].as_str();

        self.generate_docs(module_path, filter).await
      }
      "edit_file" => {
        let path = input["path"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing path"))?;
        let old_content = input["old_content"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing old_content"))?;
        let new_content = input["new_content"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing new_content"))?;

        self.edit_file(path, old_content, new_content)
      }
      "jsr_search_packages" => {
        let query = input["query"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing query"))?;
        let limit = input["limit"].as_u64().unwrap_or(20);
        let page = input["page"].as_u64().unwrap_or(1);

        self.jsr_search_packages(query, limit, page).await
      }
      "jsr_get_package" => {
        let scope = input["scope"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing scope"))?;
        let package = input["package"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing package"))?;

        self.jsr_get_package(scope, package).await
      }
      "jsr_get_package_versions" => {
        let scope = input["scope"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing scope"))?;
        let package = input["package"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing package"))?;

        self.jsr_get_package_versions(scope, package).await
      }
      "jsr_get_package_version" => {
        let scope = input["scope"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing scope"))?;
        let package = input["package"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing package"))?;
        let version = input["version"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing version"))?;

        self.jsr_get_package_version(scope, package, version).await
      }
      "jsr_get_package_dependencies" => {
        let scope = input["scope"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing scope"))?;
        let package = input["package"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing package"))?;
        let version = input["version"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing version"))?;

        self
          .jsr_get_package_dependencies(scope, package, version)
          .await
      }
      _ => {
        // Check if it's a custom tool
        if let Some(_custom_tool) =
          self.custom_tools.iter().find(|t| t.name == name)
        {
          self.execute_custom_tool(name, input).await
        } else {
          Err(AnyError::msg(format!("Unknown tool: {}", name)))
        }
      }
    };

    match &result {
      Ok(output) => {
        loader.stop(None);
      }
      Err(e) => {
        loader.error(&e.to_string());
      }
    }

    result
  }

  async fn execute_custom_tool(
    &self,
    name: &str,
    input: &serde_json::Value,
  ) -> Result<String, AnyError> {
    let config_path = self
      .custom_tools_config
      .as_ref()
      .ok_or_else(|| AnyError::msg("No custom tools config path available"))?;

    let config_specifier = ModuleSpecifier::from_file_path(config_path)
      .map_err(|_| {
        AnyError::msg("Failed to convert config path to module specifier")
      })?;

    let permissions = PermissionsContainer::allow_all(
      self.cli_factory.permission_desc_parser()?.clone(),
    );

    let mut worker = self
      .worker_factory
      .create_custom_worker(
        WorkerExecutionMode::Run,
        config_specifier,
        vec![],
        permissions,
        vec![],
        Stdio::default(),
        None,
      )
      .await
      .map_err(|e| {
        AnyError::msg(format!(
          "Failed to create worker for tool execution: {}",
          e
        ))
      })?;

    worker.execute_main_module().await.map_err(|e| {
      AnyError::msg(format!("Failed to execute custom tool: {}", e))
    })?;

    // Execute the tool using execute_script with dynamic strings
    let execute_script = format!(
      r#"
      if (!globalThis.tools) {{
        throw new Error("No globalThis.tools found. Please set globalThis.tools in your config file.");
      }}
      const tool = globalThis.tools.find(t => t.name === "{}");
      if (!tool) {{
        throw new Error("Tool not found: {}");
      }}
      if (!tool.fn) {{
        throw new Error("Tool '{}' has no function defined");
      }}
      tool.fn({})
      "#,
      name, name, name, input
    );

    // Convert the worker to MainWorker to access js_runtime
    let mut main_worker = worker.into_main_worker();
    let result_value = main_worker
      .execute_script("execute_tool", execute_script.into())
      .map_err(|e| {
        AnyError::msg(format!("Failed to execute tool '{}': {}", name, e))
      })?;

    let runtime = &mut main_worker.js_runtime;
    let scope = &mut runtime.handle_scope();
    let result_local = deno_core::v8::Local::new(scope, result_value);
    let result_json: serde_json::Value =
      deno_core::serde_v8::from_v8(scope, result_local).map_err(|e| {
        AnyError::msg(format!("Failed to deserialize result: {}", e))
      })?;

    Ok(serde_json::to_string(&result_json)?)
  }

  async fn generate_docs(
    &self,
    module_path: &str,
    filter: Option<&str>,
  ) -> Result<String, AnyError> {
    // Build the deno doc command
    let mut cmd = std::process::Command::new("deno");
    cmd.arg("doc").arg(module_path).current_dir(&self.cwd);

    // If filter is provided, add it as an additional argument
    if let Some(filter_str) = filter {
      cmd.arg(filter_str);
    }

    // Execute the command
    let output = cmd.output().map_err(|e| {
      AnyError::msg(format!("Failed to execute deno doc command: {}", e))
    })?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      return Err(AnyError::msg(format!(
        "deno doc command failed: {}",
        stderr
      )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.trim().is_empty() {
      return Ok(format!(
        "No documentation found for: {}\nMake sure the module path is correct and accessible.",
        module_path
      ));
    }

    // Return the nicely formatted output from deno doc
    let mut result = format!("Documentation for: {}\n\n", module_path);
    result.push_str(&stdout);

    if let Some(filter_str) = filter {
      result.push_str(&format!("\n\nFiltered by: '{}'", filter_str));
    }

    Ok(result)
  }

  fn edit_file(
    &self,
    path: &str,
    old_content: &str,
    new_content: &str,
  ) -> Result<String, AnyError> {
    // Read the current file content
    let current_content = fs::read_to_string(path)
      .map_err(|e| AnyError::msg(format!("Failed to read file: {}", e)))?;

    // Check if old_content exists in the file
    if !current_content.contains(old_content) {
      return Err(AnyError::msg(format!(
        "Content to replace not found in file. Please check that the old_content exactly matches what's in the file."
      )));
    }

    // Replace the content
    let new_file_content = current_content.replace(old_content, new_content);

    // Generate diff for preview
    let diff_chunks = diff(&current_content, &new_file_content);
    let mut diff_output = String::new();

    for chunk in &diff_chunks {
      match chunk {
        Chunk::Equal(text) => {
          // Only show a few lines of context around changes
          let lines: Vec<&str> = text.lines().collect();
          if lines.len() > 6 {
            for line in lines.iter().take(3) {
              diff_output.push_str(&format!("  {}\n", line));
            }
            if lines.len() > 6 {
              diff_output.push_str("  ...\n");
            }
            for line in lines.iter().skip(lines.len().saturating_sub(3)) {
              diff_output.push_str(&format!("  {}\n", line));
            }
          } else {
            for line in lines {
              diff_output.push_str(&format!("  {}\n", line));
            }
          }
        }
        Chunk::Delete(text) => {
          for line in text.lines() {
            diff_output.push_str(&format!("\x1b[31m- {}\x1b[0m\n", line));
          }
        }
        Chunk::Insert(text) => {
          for line in text.lines() {
            diff_output.push_str(&format!("\x1b[32m+ {}\x1b[0m\n", line));
          }
        }
      }
    }

    // Write the new content to the file
    if let Some(parent) = Path::new(path).parent() {
      fs::create_dir_all(parent).map_err(|e| {
        AnyError::msg(format!("Failed to create directories: {}", e))
      })?;
    }

    fs::write(path, &new_file_content)
      .map_err(|e| AnyError::msg(format!("Failed to write file: {}", e)))?;

    Ok(format!(
      "Successfully edited {}\n\nDiff:\n{}\n\nFile has been updated with the changes.",
      path, diff_output
    ))
  }

  async fn jsr_search_packages(
    &self,
    query: &str,
    limit: u64,
    page: u64,
  ) -> Result<String, AnyError> {
    let url = format!(
      "https://api.jsr.io/packages?query={}&limit={}&page={}",
      utf8_percent_encode(query, NON_ALPHANUMERIC),
      limit,
      page
    );

    let response = self.client.get(&url).send().await.map_err(|e| {
      AnyError::msg(format!("Failed to search JSR packages: {}", e))
    })?;

    if !response.status().is_success() {
      return Err(AnyError::msg(format!(
        "JSR API error: {}",
        response.status()
      )));
    }

    let body = response
      .text()
      .await
      .map_err(|e| AnyError::msg(format!("Failed to read response: {}", e)))?;

    let json: serde_json::Value = serde_json::from_str(&body)
      .map_err(|e| AnyError::msg(format!("Failed to parse JSON: {}", e)))?;

    let mut result = format!("Found JSR packages for query '{}':\n\n", query);

    if let Some(items) = json["items"].as_array() {
      for (i, item) in items.iter().enumerate() {
        let scope = item["scope"].as_str().unwrap_or("unknown");
        let name = item["name"].as_str().unwrap_or("unknown");
        let description =
          item["description"].as_str().unwrap_or("No description");
        let score = item["score"].as_f64().unwrap_or(0.0);

        result.push_str(&format!(
          "{}. @{}/{}\n   Description: {}\n   Score: {:.2}\n\n",
          i + 1,
          scope,
          name,
          description,
          score
        ));
      }

      if let Some(total) = json["total"].as_u64() {
        result.push_str(&format!("Total results: {} packages", total));
      }
    } else {
      result.push_str("No packages found.");
    }

    Ok(result)
  }

  async fn jsr_get_package(
    &self,
    scope: &str,
    package: &str,
  ) -> Result<String, AnyError> {
    let url =
      format!("https://api.jsr.io/scopes/{}/packages/{}", scope, package);

    let response = self.client.get(&url).send().await.map_err(|e| {
      AnyError::msg(format!("Failed to get JSR package: {}", e))
    })?;

    if !response.status().is_success() {
      return Err(AnyError::msg(format!(
        "JSR API error: {} - Package @{}/{} not found",
        response.status(),
        scope,
        package
      )));
    }

    let body = response
      .text()
      .await
      .map_err(|e| AnyError::msg(format!("Failed to read response: {}", e)))?;

    let json: serde_json::Value = serde_json::from_str(&body)
      .map_err(|e| AnyError::msg(format!("Failed to parse JSON: {}", e)))?;

    let mut result = format!("Package: @{}/{}\n\n", scope, package);

    if let Some(description) = json["description"].as_str() {
      result.push_str(&format!("Description: {}\n", description));
    }

    if let Some(score) = json["score"].as_f64() {
      result.push_str(&format!("Score: {:.2}\n", score));
    }

    if let Some(runtime_compat) = json["runtimeCompat"].as_object() {
      result.push_str("\nRuntime Compatibility:\n");
      if let Some(deno) = runtime_compat["deno"].as_bool() {
        result
          .push_str(&format!("  Deno: {}\n", if deno { "✅" } else { "❌" }));
      }
      if let Some(node) = runtime_compat["node"].as_bool() {
        result.push_str(&format!(
          "  Node.js: {}\n",
          if node { "✅" } else { "❌" }
        ));
      }
      if let Some(browser) = runtime_compat["browser"].as_bool() {
        result.push_str(&format!(
          "  Browser: {}\n",
          if browser { "✅" } else { "❌" }
        ));
      }
    }

    if let Some(created_at) = json["createdAt"].as_str() {
      result.push_str(&format!("\nCreated: {}\n", created_at));
    }

    if let Some(updated_at) = json["updatedAt"].as_str() {
      result.push_str(&format!("Updated: {}\n", updated_at));
    }

    if let Some(gh_repo) = json["githubRepository"].as_object() {
      if let (Some(owner), Some(repo)) =
        (gh_repo["owner"].as_str(), gh_repo["name"].as_str())
      {
        result.push_str(&format!(
          "GitHub: https://github.com/{}/{}\n",
          owner, repo
        ));
      }
    }

    Ok(result)
  }

  async fn jsr_get_package_versions(
    &self,
    scope: &str,
    package: &str,
  ) -> Result<String, AnyError> {
    let url = format!(
      "https://api.jsr.io/scopes/{}/packages/{}/versions",
      scope, package
    );

    let response = self.client.get(&url).send().await.map_err(|e| {
      AnyError::msg(format!("Failed to get JSR package versions: {}", e))
    })?;

    if !response.status().is_success() {
      return Err(AnyError::msg(format!(
        "JSR API error: {} - Package @{}/{} not found",
        response.status(),
        scope,
        package
      )));
    }

    let body = response
      .text()
      .await
      .map_err(|e| AnyError::msg(format!("Failed to read response: {}", e)))?;

    let versions: serde_json::Value = serde_json::from_str(&body)
      .map_err(|e| AnyError::msg(format!("Failed to parse JSON: {}", e)))?;

    let mut result = format!("Versions for @{}/{}:\n\n", scope, package);

    if let Some(version_list) = versions.as_array() {
      for (i, version) in version_list.iter().enumerate() {
        let version_num = version["version"].as_str().unwrap_or("unknown");
        let created_at = version["createdAt"].as_str().unwrap_or("unknown");
        let yanked = version["yanked"].as_bool().unwrap_or(false);

        result.push_str(&format!(
          "{}. {} {}\n   Created: {}\n",
          i + 1,
          version_num,
          if yanked { "(yanked)" } else { "" },
          created_at
        ));
      }
    } else {
      result.push_str("No versions found.");
    }

    Ok(result)
  }

  async fn jsr_get_package_version(
    &self,
    scope: &str,
    package: &str,
    version: &str,
  ) -> Result<String, AnyError> {
    let url = format!(
      "https://api.jsr.io/scopes/{}/packages/{}/versions/{}",
      scope, package, version
    );

    let response = self.client.get(&url).send().await.map_err(|e| {
      AnyError::msg(format!("Failed to get JSR package version: {}", e))
    })?;

    if !response.status().is_success() {
      return Err(AnyError::msg(format!(
        "JSR API error: {} - Version {} of package @{}/{} not found",
        response.status(),
        version,
        scope,
        package
      )));
    }

    let body = response
      .text()
      .await
      .map_err(|e| AnyError::msg(format!("Failed to read response: {}", e)))?;

    let json: serde_json::Value = serde_json::from_str(&body)
      .map_err(|e| AnyError::msg(format!("Failed to parse JSON: {}", e)))?;

    let mut result =
      format!("Package Version: @{}/{}@{}\n\n", scope, package, version);

    if let Some(yanked) = json["yanked"].as_bool() {
      if yanked {
        result.push_str("⚠️  This version has been yanked\n\n");
      }
    }

    if let Some(created_at) = json["createdAt"].as_str() {
      result.push_str(&format!("Created: {}\n", created_at));
    }

    if let Some(updated_at) = json["updatedAt"].as_str() {
      result.push_str(&format!("Updated: {}\n", updated_at));
    }

    if let Some(rekor_log_id) = json["rekorLogId"].as_str() {
      result.push_str(&format!("Rekor Log ID: {}\n", rekor_log_id));
    }

    Ok(result)
  }

  async fn jsr_get_package_dependencies(
    &self,
    scope: &str,
    package: &str,
    version: &str,
  ) -> Result<String, AnyError> {
    let url = format!(
      "https://api.jsr.io/scopes/{}/packages/{}/versions/{}/dependencies",
      scope, package, version
    );

    let response = self.client.get(&url).send().await.map_err(|e| {
      AnyError::msg(format!("Failed to get JSR package dependencies: {}", e))
    })?;

    if !response.status().is_success() {
      return Err(AnyError::msg(format!(
        "JSR API error: {} - Dependencies for @{}/{}@{} not found",
        response.status(),
        scope,
        package,
        version
      )));
    }

    let body = response
      .text()
      .await
      .map_err(|e| AnyError::msg(format!("Failed to read response: {}", e)))?;

    let dependencies: serde_json::Value = serde_json::from_str(&body)
      .map_err(|e| AnyError::msg(format!("Failed to parse JSON: {}", e)))?;

    let mut result =
      format!("Dependencies for @{}/{}@{}:\n\n", scope, package, version);

    if let Some(deps_list) = dependencies.as_array() {
      if deps_list.is_empty() {
        result.push_str("No dependencies found.");
      } else {
        for (i, dep) in deps_list.iter().enumerate() {
          let kind = dep["kind"].as_str().unwrap_or("unknown");
          let name = dep["name"].as_str().unwrap_or("unknown");
          let constraint = dep["constraint"].as_str().unwrap_or("unknown");
          let path = dep["path"].as_str().unwrap_or("");

          result.push_str(&format!(
            "{}. {} {}\n   Type: {}\n   Constraint: {}\n",
            i + 1,
            name,
            if path.is_empty() {
              ""
            } else {
              &format!(" ({})", path)
            },
            kind,
            constraint
          ));

          if !path.is_empty() {
            result.push_str(&format!("   Import Path: {}\n", path));
          }
          result.push('\n');
        }
      }
    } else {
      result.push_str("No dependencies found.");
    }

    Ok(result)
  }

  async fn send_message(&mut self, user_input: &str) -> Result<(), AnyError> {
    // Add user message
    self.conversation.push(AnthropicMessage {
      role: "user".to_string(),
      content: MessageContent::Text(user_input.to_string()),
    });

    loop {
      let mut api_loader =
        LoadingIndicator::new(format!("Thinking ({})", self.model_name));
      api_loader.start();

      let response_result = if self.model_provider == "anthropic" {
        self.call_anthropic().await
      } else if self.model_provider == "openai" {
        self.call_openai().await
      } else {
        api_loader.error("Unsupported AI provider");
        return Err(AnyError::msg("Unsupported AI provider"));
      };

      let response = match response_result {
        Ok(resp) => {
          api_loader.stop(Some(&format!("({})", resp.content.len())));
          resp
        }
        Err(e) => {
          api_loader.error(&e.to_string());
          return Err(e);
        }
      };

      let mut tool_calls = Vec::new();
      let mut text_response = String::new();

      for content_block in &response.content {
        match content_block.block_type.as_str() {
          "text" => {
            if let Some(text) = &content_block.text {
              text_response.push_str(text);
            }
          }
          "tool_use" => {
            if let (Some(name), Some(input), Some(id)) = (
              &content_block.name,
              &content_block.input,
              &content_block.tool_use_id,
            ) {
              tool_calls.push((id.clone(), name.clone(), input.clone()));
            }
          }
          _ => {}
        }
      }

      if !text_response.is_empty() {
        print!("\nAssistant: ");
        io::stdout().flush().ok();
        println!("{}", text_response);
      }

      // Add assistant response to conversation
      self.conversation.push(AnthropicMessage {
        role: "assistant".to_string(),
        content: MessageContent::Array(response.content),
      });

      if !tool_calls.is_empty() {
        let mut tool_results = Vec::new();

        for (tool_use_id, tool_name, tool_input) in tool_calls {
          let result = self.execute_tool(&tool_name, &tool_input).await;
          let result_text = match result {
            Ok(output) => output,
            Err(e) => {
              let error_msg = format!("Error: {}", e);
              println!("Tool error: {}", error_msg);
              error_msg
            }
          };

          tool_results.push(ContentBlock {
            block_type: "tool_result".to_string(),
            tool_use_id: Some(tool_use_id),
            text: Some(result_text),
            name: None,
            input: None,
          });
        }

        // Add tool results to conversation
        self.conversation.push(AnthropicMessage {
          role: "user".to_string(),
          content: MessageContent::Array(tool_results),
        });

        // Continue the loop to get the assistant's response to tool results
      } else {
        break;
      }
    }

    Ok(())
  }

  async fn call_anthropic(&self) -> Result<AnthropicResponse, AnyError> {
    let request = AnthropicRequest {
      model: self.model_name.clone(),
      max_tokens: 4096,
      messages: self.conversation.clone(),
      tools: Some(self.get_all_tools()),
      stream: false,
    };

    let response = self
      .client
      .post("https://api.anthropic.com/v1/messages")
      .header("x-api-key", &self.api_key)
      .header("anthropic-version", "2023-06-01")
      .header("content-type", "application/json")
      .json(&request)
      .send()
      .await
      .map_err(|e| AnyError::msg(format!("Request failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
      let error_text = response.text().await.unwrap_or_default();
      return Err(AnyError::msg(format!(
        "API request failed with status {}: {}",
        status, error_text
      )));
    }

    let ai_response: AnthropicResponse = response
      .json()
      .await
      .map_err(|e| AnyError::msg(format!("Failed to parse response: {}", e)))?;

    Ok(ai_response)
  }

  fn convert_tools_to_openai(tools: &[Tool]) -> Vec<OpenAITool> {
    tools
      .iter()
      .map(|tool| OpenAITool {
        tool_type: "function".to_string(),
        function: Function {
          name: tool.name.clone(),
          description: tool.description.clone(),
          parameters: tool.input_schema.clone(),
        },
      })
      .collect()
  }

  fn convert_conversation_to_openai(&self) -> Vec<OpenAIMessage> {
    let mut openai_messages = Vec::new();

    for msg in &self.conversation {
      match &msg.content {
        MessageContent::Text(text) => {
          openai_messages.push(OpenAIMessage {
            role: msg.role.clone(),
            content: Some(text.clone()),
            tool_calls: None,
            tool_call_id: None,
          });
        }
        MessageContent::Array(blocks) => {
          let mut text_parts = Vec::new();
          let mut tool_calls = Vec::new();

          for block in blocks {
            match block.block_type.as_str() {
              "text" => {
                if let Some(text) = &block.text {
                  text_parts.push(text.clone());
                }
              }
              "tool_use" => {
                if let (Some(name), Some(input), Some(id)) =
                  (&block.name, &block.input, &block.tool_use_id)
                {
                  tool_calls.push(ToolCall {
                    id: id.clone(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                      name: name.clone(),
                      arguments: serde_json::to_string(input)
                        .unwrap_or_default(),
                    },
                  });
                }
              }
              "tool_result" => {
                // OpenAI handles tool results differently - they go as separate messages
                if let (Some(text), Some(tool_call_id)) =
                  (&block.text, &block.tool_use_id)
                {
                  openai_messages.push(OpenAIMessage {
                    role: "tool".to_string(),
                    content: Some(text.clone()),
                    tool_calls: None,
                    tool_call_id: Some(tool_call_id.clone()),
                  });
                }
              }
              _ => {}
            }
          }

          if !text_parts.is_empty() || !tool_calls.is_empty() {
            openai_messages.push(OpenAIMessage {
              role: msg.role.clone(),
              content: if text_parts.is_empty() {
                None
              } else {
                Some(text_parts.join(""))
              },
              tool_calls: if tool_calls.is_empty() {
                None
              } else {
                Some(tool_calls)
              },
              tool_call_id: None,
            });
          }
        }
      }
    }

    openai_messages
  }

  async fn call_openai(&self) -> Result<AnthropicResponse, AnyError> {
    let openai_messages = self.convert_conversation_to_openai();
    let openai_tools = Self::convert_tools_to_openai(&self.get_all_tools());

    let request = OpenAIRequest {
      model: self.model_name.clone(),
      messages: openai_messages,
      tools: Some(openai_tools),
      stream: false,
    };

    let response = self
      .client
      .post("https://api.openai.com/v1/chat/completions")
      .header("Authorization", format!("Bearer {}", &self.api_key))
      .header("content-type", "application/json")
      .json(&request)
      .send()
      .await
      .map_err(|e| AnyError::msg(format!("Request failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
      let error_text = response.text().await.unwrap_or_default();
      return Err(AnyError::msg(format!(
        "OpenAI API request failed with status {}: {}",
        status, error_text
      )));
    }

    let openai_response: OpenAIResponse =
      response.json().await.map_err(|e| {
        AnyError::msg(format!("Failed to parse OpenAI response: {}", e))
      })?;

    // Convert OpenAI response to Anthropic format for consistency
    let mut content_blocks = Vec::new();

    if let Some(choice) = openai_response.choices.first() {
      if let Some(content) = &choice.message.content {
        content_blocks.push(ContentBlock {
          block_type: "text".to_string(),
          text: Some(content.clone()),
          tool_use_id: None,
          name: None,
          input: None,
        });
      }

      if let Some(tool_calls) = &choice.message.tool_calls {
        for tool_call in tool_calls {
          let input: serde_json::Value =
            serde_json::from_str(&tool_call.function.arguments)
              .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

          content_blocks.push(ContentBlock {
            block_type: "tool_use".to_string(),
            text: None,
            tool_use_id: Some(tool_call.id.clone()),
            name: Some(tool_call.function.name.clone()),
            input: Some(input),
          });
        }
      }
    }

    Ok(AnthropicResponse {
      content: content_blocks,
      stop_reason: Some("end_turn".to_string()),
    })
  }
}

pub async fn go(flags: Arc<Flags>, ai_flags: AiFlags) -> Result<(), AnyError> {
  println!("deno ai agent");

  // Get API configuration
  let model_provider =
    env::var("DENO_AI_PROVIDER").unwrap_or_else(|_| "openai".to_string());
  let model_name = match model_provider.as_str() {
    "anthropic" => env::var("DENO_AI_MODEL")
      .unwrap_or_else(|_| "claude-3-5-sonnet-20241022".to_string()),
    "openai" => {
      env::var("DENO_AI_MODEL").unwrap_or_else(|_| "gpt-4o".to_string())
    }
    _ => {
      return Err(AnyError::msg(
        "Unsupported AI provider. Set DENO_AI_PROVIDER to 'anthropic' or 'openai'",
      ));
    }
  };

  let api_key = match model_provider.as_str() {
    "anthropic" => env::var("ANTHROPIC_API_KEY").map_err(|_| {
      AnyError::msg("ANTHROPIC_API_KEY environment variable is required")
    })?,
    "openai" => env::var("OPENAI_API_KEY").map_err(|_| {
      AnyError::msg("OPENAI_API_KEY environment variable is required")
    })?,
    _ => unreachable!(),
  };

  println!("Using {} with model: {}", model_provider, model_name);
  println!("Type 'exit' to quit, ':help' for commands\n");

  // Create CLI factory and worker factory
  let factory = Arc::new(CliFactory::from_flags(flags));
  let worker_factory =
    Arc::new(factory.create_cli_main_worker_factory().await?);

  let custom_tools_config =
    ai_flags.config.map(|p| std::fs::canonicalize(&p).unwrap());
  let mut ai_session = AiSession::new(
    model_provider,
    model_name,
    api_key,
    custom_tools_config,
    worker_factory,
    factory,
  );

  // Load custom tools if config is provided
  if let Err(e) = ai_session.load_custom_tools().await {
    eprintln!("Warning: Failed to load custom tools: {}", e);
  }
  let mut rl = Editor::<(), DefaultHistory>::new()?;

  // Add initial system context
  ai_session.conversation.push(AnthropicMessage {
    role: "user".to_string(),
    content: MessageContent::Text(format!(
      "{}. Current working directory: {}",
      include_str!("ai.md"),
      ai_session.cwd
    )),
  });

  loop {
    let prompt_text_gray = "\x1b[90m>> \x1b[0m";
    let line = match rl.readline(prompt_text_gray) {
      Ok(line) => line,
      Err(ReadlineError::Interrupted) => {
        println!("^C");
        continue;
      }
      Err(ReadlineError::Eof) => break,
      Err(e) => {
        eprintln!("readline error: {e}");
        continue;
      }
    };

    let input = line.trim();
    if input.is_empty() {
      continue;
    }

    rl.add_history_entry(input)?;

    match input {
      "exit" | ":quit" => break,
      ":help" => {
        println!("\nAvailable commands:");
        println!(":help - Show this help message");
        println!(":quit, exit - Exit the AI assistant");
        println!("\nEnvironment variables:");
        println!(
          "DENO_AI_PROVIDER - AI provider ('anthropic' or 'openai', default: 'anthropic')"
        );
        println!(
          "DENO_AI_MODEL - Model name (default: 'claude-3-sonnet-20240229' for Anthropic)"
        );
        println!("ANTHROPIC_API_KEY - Your Anthropic API key");
        println!("OPENAI_API_KEY - Your OpenAI API key");
        println!(
          "\nAvailable tools:\n- read_file: Read file contents\n- write_file: Write/create files\n- edit_file: Edit files with diff preview\n- list_directory: List directory contents\n- execute_command: Run shell commands\n- get_docs: Generate documentation for any module using deno_doc\n- jsr_search_packages: Search for packages on JSR registry\n- jsr_get_package: Get detailed information about a JSR package\n- jsr_get_package_versions: Get all versions of a JSR package\n- jsr_get_package_version: Get details about a specific package version\n- jsr_get_package_dependencies: Get dependencies of a package version"
        );
        continue;
      }
      _ => {
        if let Err(e) = ai_session.send_message(input).await {
          eprintln!("Error: {}", e);
        }
      }
    }
  }

  println!("\nGoodbye!");
  Ok(())
}
