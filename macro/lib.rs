#![recursion_limit = "128"]
extern crate proc_macro;
#[macro_use]
extern crate quote;

use proc_macro::TokenStream;
use syn::parse_token_trees;
use syn::Lit;
use syn::StrStyle;
use syn::Token;
use syn::TokenTree;

use std::path::PathBuf;
use std::str;

#[proc_macro]
pub fn init_js(input: TokenStream) -> TokenStream {
  let directory = input.to_string();
  let args = parse_token_trees(&directory).unwrap();

  init_js_impl(args).unwrap().parse().unwrap()
}

fn init_js_impl(args: Vec<TokenTree>) -> Result<quote::Tokens, &'static str> {
  let dir: PathBuf = match args.len() {
    1 => {
      let nexttree = args.into_iter().next().unwrap();
      match nexttree {
        TokenTree::Token(Token::Literal(Lit::Str(ref val, ..))) => val.into(),
        _ => return Err("Expected string literal."),
      }
    }
    _ => return Err("Expected 1 argument."),
  };

  let listing: Vec<_> = std::fs::read_dir(dir.clone())
    .expect("Failed to read directory")
    .map(|entry| entry.unwrap().path())
    .collect();
  let mut files: Vec<&PathBuf> = listing
    .iter()
    .filter(|path| path.is_file() && path.to_string_lossy().ends_with(".js"))
    .collect();

  files.sort();

  let keys: Vec<_> = files
    .iter()
    .map(|path| {
      let path = path.strip_prefix(&dir).unwrap();
      Token::Literal(Lit::Str(
        "deno:".to_string() + &path.to_str().unwrap().to_owned(),
        StrStyle::Cooked,
      ))
    })
    .collect();

  let vals: Vec<_> = files
    .iter()
    .map(|path| {
      let path = std::fs::canonicalize(path).expect("File not found");
      Token::Literal(Lit::Str(
        path.to_str().unwrap().to_owned(),
        StrStyle::Cooked,
      ))
    })
    .collect();

  Ok(quote! {
          /// Execute this crates' JS source files.
          pub fn init(isolate: &mut JsRuntime) {
              let mut files = vec![ #((#keys, include_str!(#vals))),* ];
              for (url, source_code) in files {
                  isolate.execute(url, source_code).unwrap();
              }
          }
  })
}
