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

use std::path::Path;
use std::path::PathBuf;
use std::str;

#[proc_macro]
pub fn init_js(input: TokenStream) -> TokenStream {
  let foo = input.to_string();
  let args = parse_token_trees(&foo).unwrap();
  let gen = impl_init_js(args).unwrap();
  gen.parse().unwrap()
}

fn get_files<P: AsRef<Path>>(dir: P) -> Vec<PathBuf> {
  let mut files = vec![];
  let listing: Vec<_> = ::std::fs::read_dir(dir)
    .expect("Failed to read directory")
    .map(|entry| entry.unwrap().path())
    .collect();
  for path in listing {
    if path.is_file() && path.to_string_lossy().ends_with(".js") {
      files.push(path)
    }
  }
  files
}

fn get_path_from_args(args: Vec<TokenTree>) -> Result<PathBuf, &'static str> {
  match args.len() {
    1 => {
      let nexttree = args.into_iter().next().unwrap();
      match nexttree {
        TokenTree::Token(Token::Literal(Lit::Str(ref val, ..))) => {
          Ok(val.into())
        }
        _ => Err("Expected str."),
      }
    }
    _ => Err("Expected 1 argument."),
  }
}

fn impl_init_js(args: Vec<TokenTree>) -> Result<quote::Tokens, &'static str> {
  let dir = get_path_from_args(args)?;
  let paths: Vec<_> = get_files(&dir);

  let keys: Vec<_> = paths
    .iter()
    .map(|path| {
      let path = path.strip_prefix(&dir).unwrap();
      Token::Literal(Lit::Str(
        path.to_str().unwrap().to_owned(),
        StrStyle::Cooked,
      ))
    })
    .collect();

  let vals: Vec<_> = paths
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
