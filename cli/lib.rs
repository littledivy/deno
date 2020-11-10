
pub async fn eval_command(
    flags: Flags,
    code: String,
    as_typescript: bool,
    print: bool,
  ) -> Result<(), AnyError> {
    // Force TypeScript compile.
    let main_module =
      ModuleSpecifier::resolve_url_or_path("./$deno$eval.ts").unwrap();
    let permissions = Permissions::from_flags(&flags);
    let program_state = ProgramState::new(flags)?;
    let mut worker =
      MainWorker::new(&program_state, main_module.clone(), permissions);
    let main_module_url = main_module.as_url().to_owned();
    // Create a dummy source file.
    let source_code = if print {
      format!("console.log({})", code)
    } else {
      code
    }
    .into_bytes();
  
    let file = File {
      local: main_module_url.to_file_path().unwrap(),
      maybe_types: None,
      media_type: if as_typescript {
        MediaType::TypeScript
      } else {
        MediaType::JavaScript
      },
      source: String::from_utf8(source_code)?,
      specifier: ModuleSpecifier::from(main_module_url),
    };
  
    // Save our fake file into file fetcher cache
    // to allow module access by TS compiler.
    program_state.file_fetcher.insert_cached(file);
    debug!("main_module {}", &main_module);
    worker.execute_module(&main_module).await?;
    worker.execute("window.dispatchEvent(new Event('load'))")?;
    worker.run_event_loop().await?;
    worker.execute("window.dispatchEvent(new Event('unload'))")?;
    Ok(())
  }
  