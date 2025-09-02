// Copyright 2018-2025 the Deno authors. MIT license.

use std::sync::Arc;

use deno_core::error::AnyError;
use rustyline::{error::ReadlineError, history::DefaultHistory, Editor};

use crate::{args::Flags};

pub async fn go(
  flag: Arc<Flags>,
) -> Result<(), AnyError> {
  let mut rl = Editor::<(), DefaultHistory>::new()?;

  println!("{}", "deno ai (:help for commands)");

  loop {
    let line = match rl.readline(&format!("{}", ">> ")) {
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
  }

  Ok(())
}
