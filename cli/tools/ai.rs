// Copyright 2018-2025 the Deno authors. MIT license.

use std::env;
use std::fs;
use std::io::Write;
use std::io::{self};
use std::path::Path;
use std::sync::Arc;

use deno_core::error::AnyError;
use deno_core::serde_json;
use reqwest::Client;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use serde::Deserialize;
use serde::Serialize;

use crate::args::Flags;

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
}

struct LoadingIndicator {
  message: String,
  frames: Vec<&'static str>,
  current_frame: usize,
}

impl LoadingIndicator {
  fn new(message: String) -> Self {
    Self {
      message,
      frames: vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
      current_frame: 0,
    }
  }

  fn start(&mut self) {
    print!("\r{} {}... ", self.frames[self.current_frame], self.message);
    io::stdout().flush().ok();
    self.current_frame = (self.current_frame + 1) % self.frames.len();
  }

  fn stop(&self, result_message: Option<&str>) {
    if let Some(msg) = result_message {
      print!("\r✓ {} - {}\n", self.message, msg);
    } else {
      print!("\r✓ {}\n", self.message);
    }
    io::stdout().flush().ok();
  }

  fn error(&self, error_message: &str) {
    print!("\r✗ {} - Error: {}\n", self.message, error_message);
    io::stdout().flush().ok();
  }
}

impl AiSession {
  fn new(model_provider: String, model_name: String, api_key: String) -> Self {
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
    }
  }

  fn get_mcp_tools() -> Vec<Tool> {
    vec![
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
        name: "get_jsr_docs".to_string(),
        description: "Get comprehensive documentation for a JSR module including API reference from TOC".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "scope": {
              "type": "string",
              "description": "The JSR scope (e.g., 'std' for @std packages)"
            },
            "package": {
              "type": "string",
              "description": "The package name (e.g., 'fs')"
            },
            "version": {
              "type": "string",
              "description": "The version (optional, defaults to latest)"
            }
          },
          "required": ["scope", "package"]
        }),
      },
    ]
  }

  async fn execute_tool(
    &self,
    name: &str,
    input: &serde_json::Value,
  ) -> Result<String, AnyError> {
    let mut loader = LoadingIndicator::new(format!("Executing tool: {}", name));
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

        fs::write(path, content)
          .map_err(|e| AnyError::msg(format!("Failed to write file: {}", e)))?;
        Ok(format!("Successfully wrote to {}", path))
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
      "get_jsr_docs" => {
        let scope = input["scope"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing scope"))?;
        let package = input["package"]
          .as_str()
          .ok_or_else(|| AnyError::msg("Missing package"))?;
        let version = input["version"].as_str();

        // Get the latest version if not specified
        let version_to_use = if let Some(v) = version {
          v.to_string()
        } else {
          // First get the package info to find the latest version
          let package_url =
            format!("https://api.jsr.io/scopes/{}/packages/{}", scope, package);
          let package_response =
            self.client.get(&package_url).send().await.map_err(|e| {
              AnyError::msg(format!("Failed to fetch package info: {}", e))
            })?;

          if !package_response.status().is_success() {
            return Err(AnyError::msg(format!(
              "Package @{}/{} not found or API error",
              scope, package
            )));
          }

          // Get the versions list
          let versions_url = format!(
            "https://api.jsr.io/scopes/{}/packages/{}/versions",
            scope, package
          );
          let versions_response =
            self.client.get(&versions_url).send().await.map_err(|e| {
              AnyError::msg(format!("Failed to fetch versions: {}", e))
            })?;

          let versions: Vec<serde_json::Value> =
            versions_response.json().await.map_err(|e| {
              AnyError::msg(format!("Failed to parse versions: {}", e))
            })?;

          versions
            .first()
            .and_then(|v| v["version"].as_str())
            .unwrap_or("latest")
            .to_string()
        };

        // Get the documentation
        let docs_url = format!(
          "https://api.jsr.io/scopes/{}/packages/{}/versions/{}/docs",
          scope, package, version_to_use
        );

        let docs_response =
          self.client.get(&docs_url).send().await.map_err(|e| {
            AnyError::msg(format!("Failed to fetch docs: {}", e))
          })?;

        if !docs_response.status().is_success() {
          return Err(AnyError::msg(format!(
            "Documentation for @{}/{}@{} not found or API error",
            scope, package, version_to_use
          )));
        }

        let docs_json: serde_json::Value = docs_response
          .json()
          .await
          .map_err(|e| AnyError::msg(format!("Failed to parse docs: {}", e)))?;

        // Extract the main documentation content and TOC
        let main_docs = docs_json["main"]
          .as_str()
          .unwrap_or("No main documentation available");
        let toc = docs_json["toc"].as_str().unwrap_or("");

        // Parse the TOC to extract structured information about available symbols
        let mut api_docs = String::new();

        // The TOC contains HTML with links to different symbols and modules
        // Extract useful information from it
        if !toc.is_empty() {
          api_docs.push_str("\n\n## API Reference\n");

          // Simple HTML parsing to extract useful information
          // Look for common patterns in JSR TOC
          let lines: Vec<&str> = toc.lines().collect();

          for line in lines {
            let trimmed = line.trim();

            // Look for section headers (h2, h3, etc.)
            if trimmed.contains("<h")
              && (trimmed.contains("Functions")
                || trimmed.contains("Classes")
                || trimmed.contains("Interfaces")
                || trimmed.contains("Types")
                || trimmed.contains("Variables"))
            {
              if let Some(start) = trimmed.find('>') {
                if let Some(end) = trimmed.rfind('<') {
                  if start < end {
                    let section_title = &trimmed[start + 1..end];
                    api_docs.push_str(&format!("\n### {}\n", section_title));
                  }
                }
              }
            }

            // Look for function/class/interface definitions
            if trimmed.contains("<a href")
              && (trimmed.contains("function ")
                || trimmed.contains("class ")
                || trimmed.contains("interface ")
                || trimmed.contains("type "))
            {
              // Extract the symbol name and description
              if let Some(href_start) = trimmed.find("href=") {
                if let Some(href_end) = trimmed[href_start..].find('>') {
                  if let Some(link_end) =
                    trimmed[href_start + href_end..].find("</a>")
                  {
                    let link_content = &trimmed[href_start + href_end + 1
                      ..href_start + href_end + link_end];
                    if !link_content.trim().is_empty() {
                      api_docs
                        .push_str(&format!("- {}\n", link_content.trim()));
                    }
                  }
                }
              }
            }
          }
        }

        // If we didn't extract much from TOC, fall back to a simple structure
        if api_docs.len() < 50 && !toc.is_empty() {
          api_docs = "\n\n## API Reference\nSee the table of contents above for available functions, classes, and types.".to_string();
        }

        // Instead of truncating, let's be more selective about what we include
        // Focus on the most useful parts for coding assistance
        let mut focused_result = format!(
          "JSR Package: @{}/{} v{}\n\n",
          scope, package, version_to_use
        );

        // Extract key information from main docs (first few paragraphs, examples)
        let main_lines: Vec<&str> = main_docs.lines().collect();
        let mut included_lines = 0;
        let mut in_example = false;

        for line in main_lines.iter().take(100) {
          // Reasonable limit
          let trimmed = line.trim();

          // Always include headers, descriptions, and examples
          if trimmed.starts_with('#')
            || trimmed.starts_with("```")
            || trimmed.contains("Example")
            || trimmed.contains("Usage")
            || (included_lines < 20 && !trimmed.is_empty())
          {
            if trimmed.starts_with("```") {
              in_example = !in_example;
            }

            focused_result.push_str(line);
            focused_result.push('\n');
            included_lines += 1;
          } else if in_example {
            // Include example content
            focused_result.push_str(line);
            focused_result.push('\n');
          }

          // Stop if we have enough content and we're not in an example
          if included_lines > 50 && !in_example {
            break;
          }
        }

        // Always include the API reference if we extracted it
        if !api_docs.is_empty() {
          focused_result.push_str(&api_docs);
        }

        Ok(focused_result)
      }
      _ => Err(AnyError::msg(format!("Unknown tool: {}", name))),
    };

    match &result {
      Ok(output) => {
        let preview = if output.len() > 50 {
          format!("{}...", &output[..47])
        } else {
          output.clone()
        };
        loader.stop(Some(&preview));
      }
      Err(e) => {
        loader.error(&e.to_string());
      }
    }

    result
  }

  async fn send_message(&mut self, user_input: &str) -> Result<(), AnyError> {
    // Add user message
    self.conversation.push(AnthropicMessage {
      role: "user".to_string(),
      content: MessageContent::Text(user_input.to_string()),
    });

    loop {
      let mut api_loader = LoadingIndicator::new(format!(
        "Calling {} API ({})",
        self.model_provider.to_uppercase(),
        self.model_name
      ));
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
          api_loader.stop(Some("Response received"));
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
          println!(
            "\nExecuting tool: {} with input: {}",
            tool_name, tool_input
          );

          let result = self.execute_tool(&tool_name, &tool_input).await;
          let result_text = match result {
            Ok(output) => {
              println!("Tool output: {}", output);
              output
            }
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
      tools: Some(Self::get_mcp_tools()),
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
    let openai_tools = Self::convert_tools_to_openai(&Self::get_mcp_tools());

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

pub async fn go(_flags: Arc<Flags>) -> Result<(), AnyError> {
  println!("Deno AI - Coding Assistant");
  println!("Type 'exit' to quit, ':help' for commands\n");

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

  println!("Using {} with model: {}\n", model_provider, model_name);

  let mut ai_session = AiSession::new(model_provider, model_name, api_key);
  let mut rl = Editor::<(), DefaultHistory>::new()?;

  // Add initial system context
  ai_session.conversation.push(AnthropicMessage {
    role: "user".to_string(),
    content: MessageContent::Text(format!(
      "You are a helpful coding assistant running in Deno. The current working directory is: {}. \
       You have access to tools for reading/writing files, listing directories, and executing commands. \
       Help the user with their coding tasks.",
      ai_session.cwd
    )),
  });

  loop {
    let line = match rl.readline(">> ") {
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
          "\nAvailable tools:\n- read_file: Read file contents\n- write_file: Write/create files\n- list_directory: List directory contents\n- execute_command: Run shell commands\n- get_jsr_docs: Get documentation for JSR modules"
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
