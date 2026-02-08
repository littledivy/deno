// Copyright 2018-2026 the Deno authors. MIT license.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::collections::hash_map::Entry;
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use deno_path_util::normalize_path;
use deno_path_util::strip_unc_prefix;
use deno_runtime::colors;
use deno_runtime::deno_core::anyhow::Context;
use deno_runtime::deno_core::anyhow::bail;
use deno_runtime::deno_core::error::AnyError;
use indexmap::IndexSet;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use serde::de;
use serde::de::SeqAccess;
use serde::de::Visitor;

use crate::util::text_encoding::is_valid_utf8;

#[derive(Debug, PartialEq, Eq)]
pub enum WindowsSystemRootablePath {
  /// The root of the system above any drive letters.
  WindowSystemRoot,
  Path(PathBuf),
}

impl WindowsSystemRootablePath {
  pub fn root_for_current_os() -> Self {
    if cfg!(windows) {
      WindowsSystemRootablePath::WindowSystemRoot
    } else {
      WindowsSystemRootablePath::Path(PathBuf::from("/"))
    }
  }

  pub fn join(&self, name_component: &str) -> PathBuf {
    // this method doesn't handle multiple components
    debug_assert!(
      !name_component.contains('\\'),
      "Invalid component: {}",
      name_component
    );
    debug_assert!(
      !name_component.contains('/'),
      "Invalid component: {}",
      name_component
    );

    match self {
      WindowsSystemRootablePath::WindowSystemRoot => {
        // windows drive letter
        PathBuf::from(&format!("{}\\", name_component))
      }
      WindowsSystemRootablePath::Path(path) => path.join(name_component),
    }
  }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum FileSystemCaseSensitivity {
  #[serde(rename = "s")]
  Sensitive,
  #[serde(rename = "i")]
  Insensitive,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct VirtualDirectoryEntries(Vec<VfsEntry>);

impl VirtualDirectoryEntries {
  pub fn new(mut entries: Vec<VfsEntry>) -> Self {
    // needs to be sorted by name
    entries.sort_by(|a, b| a.name().cmp(b.name()));
    Self(entries)
  }

  pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, VfsEntry> {
    self.0.iter_mut()
  }

  pub fn iter(&self) -> std::slice::Iter<'_, VfsEntry> {
    self.0.iter()
  }

  pub fn take_inner(&mut self) -> Vec<VfsEntry> {
    std::mem::take(&mut self.0)
  }

  pub fn is_empty(&self) -> bool {
    self.0.is_empty()
  }

  pub fn len(&self) -> usize {
    self.0.len()
  }

  pub fn get_by_name(
    &self,
    name: &str,
    case_sensitivity: FileSystemCaseSensitivity,
  ) -> Option<&VfsEntry> {
    self
      .binary_search(name, case_sensitivity)
      .ok()
      .map(|index| &self.0[index])
  }

  pub fn get_mut_by_name(
    &mut self,
    name: &str,
    case_sensitivity: FileSystemCaseSensitivity,
  ) -> Option<&mut VfsEntry> {
    self
      .binary_search(name, case_sensitivity)
      .ok()
      .map(|index| &mut self.0[index])
  }

  pub fn get_mut_by_index(&mut self, index: usize) -> Option<&mut VfsEntry> {
    self.0.get_mut(index)
  }

  pub fn get_by_index(&self, index: usize) -> Option<&VfsEntry> {
    self.0.get(index)
  }

  pub fn binary_search(
    &self,
    name: &str,
    case_sensitivity: FileSystemCaseSensitivity,
  ) -> Result<usize, usize> {
    match case_sensitivity {
      FileSystemCaseSensitivity::Sensitive => {
        self.0.binary_search_by(|e| e.name().cmp(name))
      }
      FileSystemCaseSensitivity::Insensitive => self.0.binary_search_by(|e| {
        e.name()
          .chars()
          .zip(name.chars())
          .map(|(a, b)| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()))
          .find(|&ord| ord != Ordering::Equal)
          .unwrap_or_else(|| e.name().len().cmp(&name.len()))
      }),
    }
  }

  pub fn insert(
    &mut self,
    entry: VfsEntry,
    case_sensitivity: FileSystemCaseSensitivity,
  ) -> usize {
    match self.binary_search(entry.name(), case_sensitivity) {
      Ok(index) => {
        self.0[index] = entry;
        index
      }
      Err(insert_index) => {
        self.0.insert(insert_index, entry);
        insert_index
      }
    }
  }

  pub fn insert_or_modify(
    &mut self,
    name: &str,
    case_sensitivity: FileSystemCaseSensitivity,
    on_insert: impl FnOnce() -> VfsEntry,
    on_modify: impl FnOnce(&mut VfsEntry),
  ) -> usize {
    match self.binary_search(name, case_sensitivity) {
      Ok(index) => {
        on_modify(&mut self.0[index]);
        index
      }
      Err(insert_index) => {
        self.0.insert(insert_index, on_insert());
        insert_index
      }
    }
  }

  pub fn remove(&mut self, index: usize) -> VfsEntry {
    self.0.remove(index)
  }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VirtualDirectory {
  #[serde(rename = "n")]
  pub name: String,
  // should be sorted by name
  #[serde(rename = "e")]
  pub entries: VirtualDirectoryEntries,
}

#[derive(Debug, Clone, Copy)]
pub struct OffsetWithLength {
  pub offset: u64,
  pub len: u64,
}

// serialize as an array in order to save space
impl Serialize for OffsetWithLength {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    let array = [self.offset, self.len];
    array.serialize(serializer)
  }
}

impl<'de> Deserialize<'de> for OffsetWithLength {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    struct OffsetWithLengthVisitor;

    impl<'de> Visitor<'de> for OffsetWithLengthVisitor {
      type Value = OffsetWithLength;

      fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an array with two elements: [offset, len]")
      }

      fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
      where
        A: SeqAccess<'de>,
      {
        let offset = seq
          .next_element()?
          .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let len = seq
          .next_element()?
          .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        Ok(OffsetWithLength { offset, len })
      }
    }

    deserializer.deserialize_seq(OffsetWithLengthVisitor)
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualFile {
  #[serde(rename = "n")]
  pub name: String,
  #[serde(rename = "o")]
  pub offset: OffsetWithLength,
  #[serde(default, rename = "u", skip_serializing_if = "is_false")]
  pub is_valid_utf8: bool,
  #[serde(rename = "m", skip_serializing_if = "Option::is_none")]
  pub transpiled_offset: Option<OffsetWithLength>,
  #[serde(rename = "c", skip_serializing_if = "Option::is_none")]
  pub cjs_export_analysis_offset: Option<OffsetWithLength>,
  #[serde(rename = "s", skip_serializing_if = "Option::is_none")]
  pub source_map_offset: Option<OffsetWithLength>,
  #[serde(rename = "t", skip_serializing_if = "Option::is_none")]
  pub mtime: Option<u128>, // mtime in milliseconds
}

fn is_false(value: &bool) -> bool {
  !value
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VirtualSymlinkParts(Vec<String>);

impl VirtualSymlinkParts {
  pub fn from_path(path: &Path) -> Self {
    Self(
      path
        .components()
        .filter(|c| !matches!(c, std::path::Component::RootDir))
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect(),
    )
  }

  pub fn take_parts(&mut self) -> Vec<String> {
    std::mem::take(&mut self.0)
  }

  pub fn parts(&self) -> &[String] {
    &self.0
  }

  pub fn set_parts(&mut self, parts: Vec<String>) {
    self.0 = parts;
  }

  pub fn display(&self) -> String {
    self.0.join("/")
  }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VirtualSymlink {
  #[serde(rename = "n")]
  pub name: String,
  #[serde(rename = "p")]
  pub dest_parts: VirtualSymlinkParts,
}

impl VirtualSymlink {
  pub fn resolve_dest_from_root(&self, root: &Path) -> PathBuf {
    let mut dest = root.to_path_buf();
    for part in &self.dest_parts.0 {
      dest.push(part);
    }
    dest
  }
}

#[derive(Debug, Copy, Clone)]
pub enum VfsEntryRef<'a> {
  Dir(&'a VirtualDirectory),
  File(&'a VirtualFile),
  Symlink(&'a VirtualSymlink),
}

impl VfsEntryRef<'_> {
  pub fn name(&self) -> &str {
    match self {
      Self::Dir(dir) => &dir.name,
      Self::File(file) => &file.name,
      Self::Symlink(symlink) => &symlink.name,
    }
  }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum VfsEntry {
  Dir(VirtualDirectory),
  File(VirtualFile),
  Symlink(VirtualSymlink),
}

impl VfsEntry {
  pub fn name(&self) -> &str {
    match self {
      Self::Dir(dir) => &dir.name,
      Self::File(file) => &file.name,
      Self::Symlink(symlink) => &symlink.name,
    }
  }

  pub fn as_ref(&self) -> VfsEntryRef<'_> {
    match self {
      VfsEntry::Dir(dir) => VfsEntryRef::Dir(dir),
      VfsEntry::File(file) => VfsEntryRef::File(file),
      VfsEntry::Symlink(symlink) => VfsEntryRef::Symlink(symlink),
    }
  }
}

pub static DENO_COMPILE_GLOBAL_NODE_MODULES_DIR_NAME: &str =
  ".deno_compile_node_modules";

#[derive(Debug)]
pub struct BuiltVfs {
  pub root_path: WindowsSystemRootablePath,
  pub case_sensitivity: FileSystemCaseSensitivity,
  pub entries: VirtualDirectoryEntries,
  pub files: Vec<Vec<u8>>,
}

#[derive(Debug, Default)]
struct FilesData {
  files: Vec<Vec<u8>>,
  current_offset: u64,
  file_offsets: HashMap<(String, usize), OffsetWithLength>,
}

impl FilesData {
  pub fn file_bytes(&self, offset: OffsetWithLength) -> Option<&[u8]> {
    if offset.len == 0 {
      return Some(&[]);
    }

    // the debug assertions in this method should never happen
    // because it would indicate providing an offset not in the vfs
    let mut count: u64 = 0;
    for file in &self.files {
      // clippy wanted a match
      match count.cmp(&offset.offset) {
        Ordering::Equal => {
          debug_assert_eq!(offset.len, file.len() as u64);
          if offset.len == file.len() as u64 {
            return Some(file);
          } else {
            return None;
          }
        }
        Ordering::Less => {
          count += file.len() as u64;
        }
        Ordering::Greater => {
          debug_assert!(false);
          return None;
        }
      }
    }
    debug_assert!(false);
    None
  }

  pub fn add_data(&mut self, data: Vec<u8>) -> OffsetWithLength {
    if data.is_empty() {
      return OffsetWithLength { offset: 0, len: 0 };
    }
    let checksum = crate::util::checksum::r#gen(&[&data]);
    match self.file_offsets.entry((checksum, data.len())) {
      Entry::Occupied(occupied_entry) => {
        let offset_and_len = *occupied_entry.get();
        debug_assert_eq!(data.len() as u64, offset_and_len.len);
        offset_and_len
      }
      Entry::Vacant(vacant_entry) => {
        let offset_and_len = OffsetWithLength {
          offset: self.current_offset,
          len: data.len() as u64,
        };
        vacant_entry.insert(offset_and_len);
        self.current_offset += offset_and_len.len;
        self.files.push(data);
        offset_and_len
      }
    }
  }
}

pub struct AddFileDataOptions {
  pub data: Vec<u8>,
  pub mtime: Option<SystemTime>,
  pub maybe_transpiled: Option<Vec<u8>>,
  pub maybe_source_map: Option<Vec<u8>>,
  pub maybe_cjs_export_analysis: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct VfsBuilder {
  executable_root: VirtualDirectory,
  files: FilesData,
  /// The minimum root directory that should be included in the VFS.
  min_root_dir: Option<WindowsSystemRootablePath>,
  case_sensitivity: FileSystemCaseSensitivity,
  exclude_paths: HashSet<PathBuf>,
}

impl Default for VfsBuilder {
  fn default() -> Self {
    Self::new()
  }
}

impl VfsBuilder {
  pub fn new() -> Self {
    Self {
      executable_root: VirtualDirectory {
        name: "/".to_string(),
        entries: Default::default(),
      },
      files: Default::default(),
      min_root_dir: Default::default(),
      // This is not exactly correct because file systems on these OSes
      // may be case-sensitive or not based on the directory, but this
      // is a good enough approximation and limitation. In the future,
      // we may want to store this information per directory instead
      // depending on the feedback we get.
      case_sensitivity: if cfg!(windows) || cfg!(target_os = "macos") {
        FileSystemCaseSensitivity::Insensitive
      } else {
        FileSystemCaseSensitivity::Sensitive
      },
      exclude_paths: Default::default(),
    }
  }

  pub fn case_sensitivity(&self) -> FileSystemCaseSensitivity {
    self.case_sensitivity
  }

  pub fn files_len(&self) -> usize {
    self.files.files.len()
  }

  pub fn file_bytes(&self, offset: OffsetWithLength) -> Option<&[u8]> {
    self.files.file_bytes(offset)
  }

  pub fn add_exclude_path(&mut self, path: PathBuf) {
    self.exclude_paths.insert(path);
  }

  /// Add a directory that might be the minimum root directory
  /// of the VFS.
  ///
  /// For example, say the user has a deno.json and specifies an
  /// import map in a parent directory. The import map won't be
  /// included in the VFS, but its base will meaning we need to
  /// tell the VFS builder to include the base of the import map
  /// by calling this method.
  pub fn add_possible_min_root_dir(&mut self, path: &Path) {
    self.add_dir_raw(path);

    match &self.min_root_dir {
      Some(WindowsSystemRootablePath::WindowSystemRoot) => {
        // already the root dir
      }
      Some(WindowsSystemRootablePath::Path(current_path)) => {
        let mut common_components = Vec::new();
        for (a, b) in current_path.components().zip(path.components()) {
          if a != b {
            break;
          }
          common_components.push(a);
        }
        if common_components.is_empty() {
          self.min_root_dir =
            Some(WindowsSystemRootablePath::root_for_current_os());
        } else {
          self.min_root_dir = Some(WindowsSystemRootablePath::Path(
            common_components.iter().collect(),
          ));
        }
      }
      None => {
        self.min_root_dir =
          Some(WindowsSystemRootablePath::Path(path.to_path_buf()));
      }
    }
  }

  pub fn add_dir_recursive(&mut self, path: &Path) -> Result<(), AnyError> {
    let target_path = self.resolve_target_path(path)?;
    self.add_dir_recursive_not_symlink(&target_path)
  }

  fn add_dir_recursive_not_symlink(
    &mut self,
    path: &Path,
  ) -> Result<(), AnyError> {
    if self.exclude_paths.contains(path) {
      return Ok(());
    }
    self.add_dir_raw(path);
    // ok, building fs implementation
    #[allow(clippy::disallowed_methods)]
    let read_dir = std::fs::read_dir(path)
      .with_context(|| format!("Reading {}", path.display()))?;

    let mut dir_entries =
      read_dir.into_iter().collect::<Result<Vec<_>, _>>()?;
    dir_entries.sort_by_cached_key(|entry| entry.file_name()); // determinism

    for entry in dir_entries {
      let file_type = entry.file_type()?;
      let path = entry.path();
      self.add_path_with_file_type(&path, file_type)?;
    }

    Ok(())
  }

  pub fn add_path(&mut self, path: &Path) -> Result<(), AnyError> {
    // ok, building fs implementation
    #[allow(clippy::disallowed_methods)]
    let file_type = path.metadata()?.file_type();
    self.add_path_with_file_type(path, file_type)
  }

  fn add_path_with_file_type(
    &mut self,
    path: &Path,
    file_type: std::fs::FileType,
  ) -> Result<(), AnyError> {
    if self.exclude_paths.contains(path) {
      return Ok(());
    }
    if file_type.is_dir() {
      self.add_dir_recursive_not_symlink(path)
    } else if file_type.is_file() {
      self.add_file_at_path_not_symlink(path)
    } else if file_type.is_symlink() {
      match self.add_symlink(path) {
        Ok(target) => match target {
          SymlinkTarget::File(target) => {
            self.add_file_at_path_not_symlink(&target)
          }
          SymlinkTarget::Dir(target) => {
            self.add_dir_recursive_not_symlink(&target)
          }
        },
        Err(err) => {
          log::warn!(
            "{} Failed resolving symlink. Ignoring.\n    Path: {}\n    Message: {:#}",
            colors::yellow("Warning"),
            path.display(),
            err
          );
          Ok(())
        }
      }
    } else {
      // ignore
      Ok(())
    }
  }

  fn add_dir_raw(&mut self, path: &Path) -> &mut VirtualDirectory {
    log::debug!("Ensuring directory '{}'", path.display());
    debug_assert!(path.is_absolute());
    let mut current_dir = &mut self.executable_root;

    for component in path.components() {
      if matches!(component, std::path::Component::RootDir) {
        continue;
      }
      let name = component.as_os_str().to_string_lossy();
      let index = current_dir.entries.insert_or_modify(
        &name,
        self.case_sensitivity,
        || {
          VfsEntry::Dir(VirtualDirectory {
            name: name.to_string(),
            entries: Default::default(),
          })
        },
        |_| {
          // ignore
        },
      );
      match current_dir.entries.get_mut_by_index(index) {
        Some(VfsEntry::Dir(dir)) => {
          current_dir = dir;
        }
        _ => unreachable!(),
      };
    }

    current_dir
  }

  pub fn get_system_root_dir_mut(&mut self) -> &mut VirtualDirectory {
    &mut self.executable_root
  }

  pub fn get_dir_mut(&mut self, path: &Path) -> Option<&mut VirtualDirectory> {
    debug_assert!(path.is_absolute());
    let mut current_dir = &mut self.executable_root;

    for component in path.components() {
      if matches!(component, std::path::Component::RootDir) {
        continue;
      }
      let name = component.as_os_str().to_string_lossy();
      let entry = current_dir
        .entries
        .get_mut_by_name(&name, self.case_sensitivity)?;
      match entry {
        VfsEntry::Dir(dir) => {
          current_dir = dir;
        }
        _ => unreachable!("{}", path.display()),
      };
    }

    Some(current_dir)
  }

  pub fn add_file_at_path(&mut self, path: &Path) -> Result<(), AnyError> {
    if self.exclude_paths.contains(path) {
      return Ok(());
    }
    let (file_bytes, mtime) = self.read_file_bytes_and_mtime(path)?;
    self.add_file_with_data(
      path,
      AddFileDataOptions {
        data: file_bytes,
        mtime,
        maybe_cjs_export_analysis: None,
        maybe_transpiled: None,
        maybe_source_map: None,
      },
    )
  }

  fn add_file_at_path_not_symlink(
    &mut self,
    path: &Path,
  ) -> Result<(), AnyError> {
    if self.exclude_paths.contains(path) {
      return Ok(());
    }
    let (file_bytes, mtime) = self.read_file_bytes_and_mtime(path)?;
    self.add_file_with_data_raw(path, file_bytes, mtime)
  }

  fn read_file_bytes_and_mtime(
    &self,
    path: &Path,
  ) -> Result<(Vec<u8>, Option<SystemTime>), AnyError> {
    // ok, building fs implementation
    #[allow(clippy::disallowed_methods)]
    {
      let mut file = std::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("Opening {}", path.display()))?;
      let mtime = file.metadata().ok().and_then(|m| m.modified().ok());
      let mut file_bytes = Vec::new();
      file
        .read_to_end(&mut file_bytes)
        .with_context(|| format!("Reading {}", path.display()))?;
      Ok((file_bytes, mtime))
    }
  }

  pub fn add_file_with_data(
    &mut self,
    path: &Path,
    options: AddFileDataOptions,
  ) -> Result<(), AnyError> {
    // ok, fs implementation
    #[allow(clippy::disallowed_methods)]
    let metadata = std::fs::symlink_metadata(path).with_context(|| {
      format!("Resolving target path for '{}'", path.display())
    })?;
    if metadata.is_symlink() {
      let target = self.add_symlink(path)?.into_path_buf();
      self.add_file_with_data_raw_options(&target, options)
    } else {
      self.add_file_with_data_raw_options(path, options)
    }
  }

  pub fn add_file_with_data_raw(
    &mut self,
    path: &Path,
    data: Vec<u8>,
    mtime: Option<SystemTime>,
  ) -> Result<(), AnyError> {
    self.add_file_with_data_raw_options(
      path,
      AddFileDataOptions {
        data,
        mtime,
        maybe_transpiled: None,
        maybe_cjs_export_analysis: None,
        maybe_source_map: None,
      },
    )
  }

  fn add_file_with_data_raw_options(
    &mut self,
    path: &Path,
    options: AddFileDataOptions,
  ) -> Result<(), AnyError> {
    log::debug!("Adding file '{}'", path.display());
    let case_sensitivity = self.case_sensitivity;

    let is_valid_utf8 = is_valid_utf8(&options.data);
    let offset_and_len = self.files.add_data(options.data);
    let transpiled_offset = options
      .maybe_transpiled
      .map(|data| self.files.add_data(data));
    let source_map_offset = options
      .maybe_source_map
      .map(|data| self.files.add_data(data));
    let cjs_export_analysis_offset = options
      .maybe_cjs_export_analysis
      .map(|data| self.files.add_data(data));
    let dir = self.add_dir_raw(path.parent().unwrap());
    let name = path.file_name().unwrap().to_string_lossy();

    let mtime = options
      .mtime
      .and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
      .map(|m| m.as_millis());

    dir.entries.insert_or_modify(
      &name,
      case_sensitivity,
      || {
        VfsEntry::File(VirtualFile {
          name: name.to_string(),
          is_valid_utf8,
          offset: offset_and_len,
          transpiled_offset,
          cjs_export_analysis_offset,
          source_map_offset,
          mtime,
        })
      },
      |entry| match entry {
        VfsEntry::File(virtual_file) => {
          virtual_file.offset = offset_and_len;
          // doesn't overwrite to None
          if transpiled_offset.is_some() {
            virtual_file.transpiled_offset = transpiled_offset;
          }
          if source_map_offset.is_some() {
            virtual_file.source_map_offset = source_map_offset;
          }
          if cjs_export_analysis_offset.is_some() {
            virtual_file.cjs_export_analysis_offset =
              cjs_export_analysis_offset;
          }
          virtual_file.mtime = mtime;
        }
        VfsEntry::Dir(_) | VfsEntry::Symlink(_) => unreachable!(),
      },
    );

    Ok(())
  }

  fn resolve_target_path(&mut self, path: &Path) -> Result<PathBuf, AnyError> {
    // ok, fs implementation
    #[allow(clippy::disallowed_methods)]
    let metadata = std::fs::symlink_metadata(path).with_context(|| {
      format!("Resolving target path for '{}'", path.display())
    })?;
    if metadata.is_symlink() {
      Ok(self.add_symlink(path)?.into_path_buf())
    } else {
      Ok(path.to_path_buf())
    }
  }

  pub fn add_symlink(
    &mut self,
    path: &Path,
  ) -> Result<SymlinkTarget, AnyError> {
    self.add_symlink_inner(path, &mut IndexSet::new())
  }

  fn add_symlink_inner(
    &mut self,
    path: &Path,
    visited: &mut IndexSet<PathBuf>,
  ) -> Result<SymlinkTarget, AnyError> {
    log::debug!("Adding symlink '{}'", path.display());
    let target = strip_unc_prefix(
      // ok, fs implementation
      #[allow(clippy::disallowed_methods)]
      std::fs::read_link(path)
        .with_context(|| format!("Reading symlink '{}'", path.display()))?,
    );
    let case_sensitivity = self.case_sensitivity;
    let target =
      normalize_path(Cow::Owned(path.parent().unwrap().join(&target)));
    let dir = self.add_dir_raw(path.parent().unwrap());
    let name = path.file_name().unwrap().to_string_lossy();
    dir.entries.insert_or_modify(
      &name,
      case_sensitivity,
      || {
        VfsEntry::Symlink(VirtualSymlink {
          name: name.to_string(),
          dest_parts: VirtualSymlinkParts::from_path(&target),
        })
      },
      |_| {
        // ignore previously inserted
      },
    );
    // ok, fs implementation
    #[allow(clippy::disallowed_methods)]
    let target_metadata =
      std::fs::symlink_metadata(&target).with_context(|| {
        format!("Reading symlink target '{}'", target.display())
      })?;
    if target_metadata.is_symlink() {
      if !visited.insert(target.to_path_buf()) {
        // todo: probably don't error in this scenario
        bail!(
          "Circular symlink detected: {} -> {}",
          visited
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> "),
          target.display()
        );
      }
      self.add_symlink_inner(&target, visited)
    } else if target_metadata.is_dir() {
      Ok(SymlinkTarget::Dir(target.into_owned()))
    } else {
      Ok(SymlinkTarget::File(target.into_owned()))
    }
  }

  /// Adds the CJS export analysis to the provided file.
  ///
  /// Warning: This will panic if the file wasn't properly
  /// setup before calling this.
  pub fn add_cjs_export_analysis(&mut self, path: &Path, data: Vec<u8>) {
    self.add_data_for_file_or_panic(path, data, |file, offset_with_length| {
      file.cjs_export_analysis_offset = Some(offset_with_length);
    })
  }

  fn add_data_for_file_or_panic(
    &mut self,
    path: &Path,
    data: Vec<u8>,
    update_file: impl FnOnce(&mut VirtualFile, OffsetWithLength),
  ) {
    let offset_with_length = self.files.add_data(data);
    let case_sensitivity = self.case_sensitivity;
    let dir = self.get_dir_mut(path.parent().unwrap()).unwrap();
    let name = path.file_name().unwrap().to_string_lossy();
    let file = dir
      .entries
      .get_mut_by_name(&name, case_sensitivity)
      .unwrap();
    match file {
      VfsEntry::File(virtual_file) => {
        update_file(virtual_file, offset_with_length);
      }
      VfsEntry::Dir(_) | VfsEntry::Symlink(_) => {
        unreachable!()
      }
    }
  }

  /// Iterates through all the files in the virtual file system.
  pub fn iter_files(
    &self,
  ) -> impl Iterator<Item = (PathBuf, &VirtualFile)> + '_ {
    FileIterator {
      pending_dirs: VecDeque::from([(
        WindowsSystemRootablePath::root_for_current_os(),
        &self.executable_root,
      )]),
      current_dir_index: 0,
    }
  }

  pub fn build(self) -> BuiltVfs {
    fn strip_prefix_from_symlinks(
      dir: &mut VirtualDirectory,
      parts: &[String],
    ) {
      for entry in dir.entries.iter_mut() {
        match entry {
          VfsEntry::Dir(dir) => {
            strip_prefix_from_symlinks(dir, parts);
          }
          VfsEntry::File(_) => {}
          VfsEntry::Symlink(symlink) => {
            let parts = symlink
              .dest_parts
              .take_parts()
              .into_iter()
              .skip(parts.len())
              .collect();
            symlink.dest_parts.set_parts(parts);
          }
        }
      }
    }

    let mut current_dir = self.executable_root;
    let mut current_path = WindowsSystemRootablePath::root_for_current_os();
    loop {
      if current_dir.entries.len() != 1 {
        break;
      }
      if self.min_root_dir.as_ref() == Some(&current_path) {
        break;
      }
      match current_dir.entries.iter().next().unwrap() {
        VfsEntry::Dir(dir) => {
          if dir.name == DENO_COMPILE_GLOBAL_NODE_MODULES_DIR_NAME {
            // special directory we want to maintain
            break;
          }
          match current_dir.entries.remove(0) {
            VfsEntry::Dir(dir) => {
              current_path =
                WindowsSystemRootablePath::Path(current_path.join(&dir.name));
              current_dir = dir;
            }
            _ => unreachable!(),
          };
        }
        VfsEntry::File(_) | VfsEntry::Symlink(_) => break,
      }
    }
    if let WindowsSystemRootablePath::Path(path) = &current_path {
      strip_prefix_from_symlinks(
        &mut current_dir,
        VirtualSymlinkParts::from_path(path).parts(),
      );
    }
    BuiltVfs {
      root_path: current_path,
      case_sensitivity: self.case_sensitivity,
      entries: current_dir.entries,
      files: self.files.files,
    }
  }
}

struct FileIterator<'a> {
  pending_dirs: VecDeque<(WindowsSystemRootablePath, &'a VirtualDirectory)>,
  current_dir_index: usize,
}

impl<'a> Iterator for FileIterator<'a> {
  type Item = (PathBuf, &'a VirtualFile);

  fn next(&mut self) -> Option<Self::Item> {
    while !self.pending_dirs.is_empty() {
      let (dir_path, current_dir) = self.pending_dirs.front()?;
      if let Some(entry) =
        current_dir.entries.get_by_index(self.current_dir_index)
      {
        self.current_dir_index += 1;
        match entry {
          VfsEntry::Dir(virtual_directory) => {
            self.pending_dirs.push_back((
              WindowsSystemRootablePath::Path(
                dir_path.join(&virtual_directory.name),
              ),
              virtual_directory,
            ));
          }
          VfsEntry::File(virtual_file) => {
            return Some((dir_path.join(&virtual_file.name), virtual_file));
          }
          VfsEntry::Symlink(_) => {
            // ignore
          }
        }
      } else {
        self.pending_dirs.pop_front();
        self.current_dir_index = 0;
      }
    }
    None
  }
}

#[derive(Debug)]
pub enum SymlinkTarget {
  File(PathBuf),
  Dir(PathBuf),
}

impl SymlinkTarget {
  pub fn into_path_buf(self) -> PathBuf {
    match self {
      Self::File(path) => path,
      Self::Dir(path) => path,
    }
  }
}

// ============================================================================
// Binary VFS format — zero-copy types and serialization
// ============================================================================

/// Magic bytes identifying the binary VFS format.
pub const VFS_MAGIC: [u8; 4] = *b"VFS1";

/// Entry type discriminants in the flat entry array.
const ENTRY_TYPE_DIR: u8 = 0;
const ENTRY_TYPE_FILE: u8 = 1;
const ENTRY_TYPE_SYMLINK: u8 = 2;

/// Flag bit: file contents are valid UTF-8.
const FLAG_VALID_UTF8: u8 = 1 << 0;

/// Fixed size of a single entry in the flat array: 80 bytes.
const RAW_ENTRY_SIZE: usize = 80;

/// Binary VFS header, 24 bytes.
///
/// ```text
/// magic:                [u8; 4]  = "VFS1"
/// string_table_len:     u32
/// string_count:         u32
/// entry_count:          u32
/// root_children_start:  u32
/// root_children_count:  u32
/// ```
const HEADER_SIZE: usize = 24;

// StringRef is the conceptual format of the string index entries (8 bytes each:
// offset: u32, len: u32). They are read directly from raw bytes at runtime
// rather than being instantiated as Rust structs.

// -- Write-side: serialize a `VirtualDirectoryEntries` tree into binary --

/// Serialize a `VirtualDirectoryEntries` tree (+ root name) into the
/// binary VFS format. Returns the serialized bytes.
pub fn serialize_vfs_binary(entries: &VirtualDirectoryEntries) -> Vec<u8> {
  struct Ctx {
    strings: Vec<(u32, u32)>, // (offset, len) pairs
    string_data: Vec<u8>,
    string_map: HashMap<String, u32>, // dedup: string → id
    entries: Vec<[u8; RAW_ENTRY_SIZE]>,
    symlink_parts: Vec<u32>,
  }

  impl Ctx {
    fn intern_string(&mut self, s: &str) -> u32 {
      if let Some(&id) = self.string_map.get(s) {
        return id;
      }
      let id = self.strings.len() as u32;
      let offset = self.string_data.len() as u32;
      let len = s.len() as u32;
      self.string_data.extend_from_slice(s.as_bytes());
      self.strings.push((offset, len));
      self.string_map.insert(s.to_string(), id);
      id
    }

  }

  fn write_entries(ctx: &mut Ctx, entries: &VirtualDirectoryEntries) -> (u32, u32) {
    // Allocate slots for children first so they are contiguous
    let count = entries.len() as u32;
    let start = ctx.entries.len() as u32;
    // Reserve slots
    for _ in 0..count {
      ctx.entries.push([0u8; RAW_ENTRY_SIZE]);
    }
    // Fill each slot
    for (i, entry) in entries.iter().enumerate() {
      let slot_idx = start + i as u32;
      let buf = match entry {
        VfsEntry::Dir(dir) => {
          let name_id = ctx.intern_string(&dir.name);
          let (ch_start, ch_count) = write_entries(ctx, &dir.entries);
          make_dir_entry(name_id, ch_start, ch_count)
        }
        VfsEntry::File(file) => {
          let name_id = ctx.intern_string(&file.name);
          make_file_entry(name_id, file)
        }
        VfsEntry::Symlink(symlink) => {
          let name_id = ctx.intern_string(&symlink.name);
          let parts_start = ctx.symlink_parts.len() as u32;
          let parts_count = symlink.dest_parts.parts().len() as u32;
          for part in symlink.dest_parts.parts() {
            let part_id = ctx.intern_string(part);
            ctx.symlink_parts.push(part_id);
          }
          make_symlink_entry(name_id, parts_start, parts_count)
        }
      };
      ctx.entries[slot_idx as usize] = buf;
    }
    (start, count)
  }

  let mut ctx = Ctx {
    strings: Vec::new(),
    string_data: Vec::new(),
    string_map: HashMap::new(),
    entries: Vec::new(),
    symlink_parts: Vec::new(),
  };

  let (root_children_start, root_children_count) = write_entries(&mut ctx, entries);

  // Compute output size
  let string_index_size = ctx.strings.len() * 8; // 8 bytes per StringRef
  let entries_size = ctx.entries.len() * RAW_ENTRY_SIZE;
  let symlink_parts_size = ctx.symlink_parts.len() * 4;
  let total = HEADER_SIZE
    + string_index_size
    + ctx.string_data.len()
    + symlink_parts_size
    + 4 // symlink_parts_count
    + entries_size;

  let mut out = Vec::with_capacity(total);

  // Header
  out.extend_from_slice(&VFS_MAGIC);
  out.extend_from_slice(&(ctx.string_data.len() as u32).to_le_bytes());
  out.extend_from_slice(&(ctx.strings.len() as u32).to_le_bytes());
  out.extend_from_slice(&(ctx.entries.len() as u32).to_le_bytes());
  out.extend_from_slice(&root_children_start.to_le_bytes());
  out.extend_from_slice(&root_children_count.to_le_bytes());

  // String index
  for &(offset, len) in &ctx.strings {
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
  }

  // String data
  out.extend_from_slice(&ctx.string_data);

  // Symlink parts array
  for &part_id in &ctx.symlink_parts {
    out.extend_from_slice(&part_id.to_le_bytes());
  }
  out.extend_from_slice(&(ctx.symlink_parts.len() as u32).to_le_bytes());

  // Entry array
  for entry_buf in &ctx.entries {
    out.extend_from_slice(entry_buf);
  }

  debug_assert_eq!(out.len(), total);
  out
}

fn make_dir_entry(name_id: u32, children_start: u32, children_count: u32) -> [u8; RAW_ENTRY_SIZE] {
  let mut buf = [0u8; RAW_ENTRY_SIZE];
  buf[0] = ENTRY_TYPE_DIR;
  // flags=0, pad=0
  buf[4..8].copy_from_slice(&name_id.to_le_bytes());
  // Union bytes start at offset 8
  buf[8..12].copy_from_slice(&children_start.to_le_bytes());
  buf[12..16].copy_from_slice(&children_count.to_le_bytes());
  buf
}

fn make_file_entry(name_id: u32, file: &VirtualFile) -> [u8; RAW_ENTRY_SIZE] {
  let mut buf = [0u8; RAW_ENTRY_SIZE];
  buf[0] = ENTRY_TYPE_FILE;
  let mut flags: u8 = 0;
  if file.is_valid_utf8 {
    flags |= FLAG_VALID_UTF8;
  }
  buf[1] = flags;
  buf[4..8].copy_from_slice(&name_id.to_le_bytes());

  // Union bytes at offset 8:
  // data_offset: u64 @8
  // data_len: u64 @16
  // transpiled_offset: u64 @24
  // transpiled_len: u64 @32
  // source_map_offset: u64 @40
  // source_map_len: u64 @48
  // cjs_export_offset: u64 @56
  // cjs_export_len: u64 @64
  // mtime_ms: u64 @72
  buf[8..16].copy_from_slice(&file.offset.offset.to_le_bytes());
  buf[16..24].copy_from_slice(&file.offset.len.to_le_bytes());

  if let Some(t) = &file.transpiled_offset {
    buf[24..32].copy_from_slice(&t.offset.to_le_bytes());
    buf[32..40].copy_from_slice(&t.len.to_le_bytes());
  }
  if let Some(s) = &file.source_map_offset {
    buf[40..48].copy_from_slice(&s.offset.to_le_bytes());
    buf[48..56].copy_from_slice(&s.len.to_le_bytes());
  }
  if let Some(c) = &file.cjs_export_analysis_offset {
    buf[56..64].copy_from_slice(&c.offset.to_le_bytes());
    buf[64..72].copy_from_slice(&c.len.to_le_bytes());
  }
  // mtime: store as u64 milliseconds (0 = absent)
  let mtime_ms: u64 = file
    .mtime
    .map(|m| m.try_into().unwrap_or(u64::MAX))
    .unwrap_or(0);
  buf[72..80].copy_from_slice(&mtime_ms.to_le_bytes());

  buf
}

fn make_symlink_entry(name_id: u32, parts_start: u32, parts_count: u32) -> [u8; RAW_ENTRY_SIZE] {
  let mut buf = [0u8; RAW_ENTRY_SIZE];
  buf[0] = ENTRY_TYPE_SYMLINK;
  buf[4..8].copy_from_slice(&name_id.to_le_bytes());
  buf[8..12].copy_from_slice(&parts_start.to_le_bytes());
  buf[12..16].copy_from_slice(&parts_count.to_le_bytes());
  buf
}

// -- Read-side: zero-copy view over mmap'd binary VFS data --

/// Zero-copy view over the binary VFS section. All data is borrowed
/// from the mmap'd section — no heap allocations.
#[derive(Debug)]
pub struct BinaryVfsView {
  /// Byte slice containing the string index entries (8 bytes each).
  string_index_data: &'static [u8],
  /// Concatenated UTF-8 string data.
  string_data: &'static [u8],
  /// Byte slice of the flat entry array (RAW_ENTRY_SIZE bytes each).
  entries_data: &'static [u8],
  /// Symlink parts (string ids as u32 LE).
  symlink_parts_data: &'static [u8],
  /// Number of entries in the flat array.
  _entry_count: u32,
  /// Root directory children range.
  pub root_children_start: u32,
  pub root_children_count: u32,
}

/// A reference to a single entry in the binary VFS.
#[derive(Debug, Clone, Copy)]
pub struct BinaryVfsEntry<'a> {
  data: &'a [u8], // exactly RAW_ENTRY_SIZE bytes
  vfs: &'a BinaryVfsView,
}

/// What kind of entry this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryVfsEntryKind {
  Dir,
  File,
  Symlink,
}

impl BinaryVfsView {
  /// Parse a binary VFS from the given data slice.
  /// Returns `(remaining_input, view)`.
  pub fn from_bytes(data: &'static [u8]) -> Result<(&'static [u8], Self), std::io::Error> {
    if data.len() < HEADER_SIZE {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "binary VFS data too short for header",
      ));
    }

    // Parse header
    let magic = &data[0..4];
    if magic != VFS_MAGIC {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid VFS magic bytes",
      ));
    }
    let string_table_len = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let string_count = u32::from_le_bytes(data[8..12].try_into().unwrap());
    let entry_count = u32::from_le_bytes(data[12..16].try_into().unwrap());
    let root_children_start = u32::from_le_bytes(data[16..20].try_into().unwrap());
    let root_children_count = u32::from_le_bytes(data[20..24].try_into().unwrap());

    let mut pos = HEADER_SIZE;

    // String index
    let string_index_size = string_count as usize * 8;
    if data.len() < pos + string_index_size {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "binary VFS data too short for string index",
      ));
    }
    let string_index_data = &data[pos..pos + string_index_size];
    pos += string_index_size;

    // String data
    let stl = string_table_len as usize;
    if data.len() < pos + stl {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "binary VFS data too short for string data",
      ));
    }
    let string_data = &data[pos..pos + stl];
    pos += stl;

    // Symlink parts array + count
    // The symlink_parts_count u32 comes AFTER the parts array.
    // But we need the count to know how many parts there are.
    // We stored it after the parts, so we need to scan ahead.
    // Actually, looking at the write side: we write parts then count.
    // So read count from the end of the parts section:
    // We need to find the symlink_parts_count first. It's stored as a u32
    // right after the symlink parts array. But we don't know the array
    // length yet. We stored it AT THE END of the parts section.
    //
    // Layout: [parts...][symlink_parts_count: u32][entries...]
    // Since entries are at the end, and we know entry_count and total
    // remaining length, we can compute:
    let entries_total_size = entry_count as usize * RAW_ENTRY_SIZE;
    let remaining_before_entries = data.len() - pos - entries_total_size;
    // remaining_before_entries = symlink_parts_data + 4 (for count)
    if remaining_before_entries < 4 {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "binary VFS data too short for symlink parts count",
      ));
    }
    let symlink_parts_count_offset = pos + remaining_before_entries - 4;
    let symlink_parts_count = u32::from_le_bytes(
      data[symlink_parts_count_offset..symlink_parts_count_offset + 4]
        .try_into()
        .unwrap(),
    );
    let symlink_parts_byte_len = symlink_parts_count as usize * 4;
    if remaining_before_entries != symlink_parts_byte_len + 4 {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "binary VFS symlink parts size mismatch",
      ));
    }
    let symlink_parts_data = &data[pos..pos + symlink_parts_byte_len];
    pos += symlink_parts_byte_len + 4; // skip parts + count

    // Entry array
    if data.len() < pos + entries_total_size {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "binary VFS data too short for entries",
      ));
    }
    let entries_data = &data[pos..pos + entries_total_size];
    pos += entries_total_size;

    Ok((
      &data[pos..],
      Self {
        string_index_data,
        string_data,
        entries_data,
        symlink_parts_data,
        _entry_count: entry_count,
        root_children_start,
        root_children_count,
      },
    ))
  }

  /// Look up a string by its ID (index into string table).
  pub fn get_string(&self, id: u32) -> &'static str {
    let idx_offset = id as usize * 8;
    let offset = u32::from_le_bytes(
      self.string_index_data[idx_offset..idx_offset + 4]
        .try_into()
        .unwrap(),
    ) as usize;
    let len = u32::from_le_bytes(
      self.string_index_data[idx_offset + 4..idx_offset + 8]
        .try_into()
        .unwrap(),
    ) as usize;
    // SAFETY: the serializer wrote valid UTF-8 strings
    unsafe { std::str::from_utf8_unchecked(&self.string_data[offset..offset + len]) }
  }

  /// Get an entry by its index in the flat array.
  pub fn get_entry(&self, index: u32) -> BinaryVfsEntry<'_> {
    let start = index as usize * RAW_ENTRY_SIZE;
    BinaryVfsEntry {
      data: &self.entries_data[start..start + RAW_ENTRY_SIZE],
      vfs: self,
    }
  }

  /// Get a slice of contiguous entries (e.g. children of a directory).
  pub fn get_entries(&self, start: u32, count: u32) -> BinaryVfsEntries<'_> {
    BinaryVfsEntries {
      vfs: self,
      start,
      count,
    }
  }

  /// Get the root children as an entries iterator.
  pub fn root_children(&self) -> BinaryVfsEntries<'_> {
    self.get_entries(self.root_children_start, self.root_children_count)
  }

  /// Get symlink destination parts (as string IDs).
  fn get_symlink_part_string_id(&self, index: u32) -> u32 {
    let offset = index as usize * 4;
    u32::from_le_bytes(
      self.symlink_parts_data[offset..offset + 4]
        .try_into()
        .unwrap(),
    )
  }

  /// Binary search within a range of children for a name.
  pub fn binary_search(
    &self,
    children_start: u32,
    children_count: u32,
    name: &str,
    case_sensitivity: FileSystemCaseSensitivity,
  ) -> Result<u32, u32> {
    let mut low = 0u32;
    let mut high = children_count;
    while low < high {
      let mid = low + (high - low) / 2;
      let entry = self.get_entry(children_start + mid);
      let entry_name = entry.name();
      let cmp = match case_sensitivity {
        FileSystemCaseSensitivity::Sensitive => entry_name.cmp(name),
        FileSystemCaseSensitivity::Insensitive => entry_name
          .chars()
          .zip(name.chars())
          .map(|(a, b)| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()))
          .find(|&ord| ord != Ordering::Equal)
          .unwrap_or_else(|| entry_name.len().cmp(&name.len())),
      };
      match cmp {
        Ordering::Less => low = mid + 1,
        Ordering::Greater => high = mid,
        Ordering::Equal => return Ok(children_start + mid),
      }
    }
    Err(low)
  }

  /// Convenience: look up an entry by name within a parent's children.
  pub fn get_child_by_name(
    &self,
    children_start: u32,
    children_count: u32,
    name: &str,
    case_sensitivity: FileSystemCaseSensitivity,
  ) -> Option<BinaryVfsEntry<'_>> {
    self
      .binary_search(children_start, children_count, name, case_sensitivity)
      .ok()
      .map(|idx| self.get_entry(idx))
  }
}

/// A contiguous range of entries in the binary VFS.
#[derive(Debug, Clone)]
pub struct BinaryVfsEntries<'a> {
  vfs: &'a BinaryVfsView,
  start: u32,
  count: u32,
}

impl<'a> BinaryVfsEntries<'a> {
  pub fn len(&self) -> usize {
    self.count as usize
  }

  pub fn is_empty(&self) -> bool {
    self.count == 0
  }

  pub fn get(&self, index: usize) -> Option<BinaryVfsEntry<'a>> {
    if index < self.count as usize {
      Some(self.vfs.get_entry(self.start + index as u32))
    } else {
      None
    }
  }

  pub fn iter(&self) -> BinaryVfsEntriesIter<'a> {
    BinaryVfsEntriesIter {
      vfs: self.vfs,
      current: self.start,
      end: self.start + self.count,
    }
  }

  pub fn get_by_name(
    &self,
    name: &str,
    case_sensitivity: FileSystemCaseSensitivity,
  ) -> Option<BinaryVfsEntry<'a>> {
    self.vfs.get_child_by_name(
      self.start,
      self.count,
      name,
      case_sensitivity,
    )
  }
}

impl<'a> IntoIterator for &'a BinaryVfsEntries<'a> {
  type Item = BinaryVfsEntry<'a>;
  type IntoIter = BinaryVfsEntriesIter<'a>;
  fn into_iter(self) -> Self::IntoIter {
    self.iter()
  }
}

pub struct BinaryVfsEntriesIter<'a> {
  vfs: &'a BinaryVfsView,
  current: u32,
  end: u32,
}

impl<'a> Iterator for BinaryVfsEntriesIter<'a> {
  type Item = BinaryVfsEntry<'a>;
  fn next(&mut self) -> Option<Self::Item> {
    if self.current >= self.end {
      return None;
    }
    let entry = self.vfs.get_entry(self.current);
    self.current += 1;
    Some(entry)
  }

  fn size_hint(&self) -> (usize, Option<usize>) {
    let remaining = (self.end - self.current) as usize;
    (remaining, Some(remaining))
  }
}

impl ExactSizeIterator for BinaryVfsEntriesIter<'_> {}

impl<'a> BinaryVfsEntry<'a> {
  /// The kind of this entry.
  pub fn kind(&self) -> BinaryVfsEntryKind {
    match self.data[0] {
      ENTRY_TYPE_DIR => BinaryVfsEntryKind::Dir,
      ENTRY_TYPE_FILE => BinaryVfsEntryKind::File,
      ENTRY_TYPE_SYMLINK => BinaryVfsEntryKind::Symlink,
      _ => panic!("invalid VFS entry type"),
    }
  }

  fn name_id(&self) -> u32 {
    u32::from_le_bytes(self.data[4..8].try_into().unwrap())
  }

  /// The name of this entry.
  pub fn name(&self) -> &'static str {
    self.vfs.get_string(self.name_id())
  }

  /// For directories: children start index.
  pub fn dir_children_start(&self) -> u32 {
    debug_assert_eq!(self.kind(), BinaryVfsEntryKind::Dir);
    u32::from_le_bytes(self.data[8..12].try_into().unwrap())
  }

  /// For directories: children count.
  pub fn dir_children_count(&self) -> u32 {
    debug_assert_eq!(self.kind(), BinaryVfsEntryKind::Dir);
    u32::from_le_bytes(self.data[12..16].try_into().unwrap())
  }

  /// For directories: get children as BinaryVfsEntries.
  pub fn dir_children(&self) -> BinaryVfsEntries<'a> {
    self.vfs.get_entries(self.dir_children_start(), self.dir_children_count())
  }

  /// For files: is_valid_utf8 flag.
  pub fn file_is_valid_utf8(&self) -> bool {
    self.data[1] & FLAG_VALID_UTF8 != 0
  }

  fn read_u64_at(&self, offset: usize) -> u64 {
    u64::from_le_bytes(self.data[offset..offset + 8].try_into().unwrap())
  }

  /// For files: data offset and length.
  pub fn file_data_offset(&self) -> OffsetWithLength {
    OffsetWithLength {
      offset: self.read_u64_at(8),
      len: self.read_u64_at(16),
    }
  }

  /// For files: transpiled offset and length (None if len==0).
  pub fn file_transpiled_offset(&self) -> Option<OffsetWithLength> {
    let o = self.read_u64_at(24);
    let l = self.read_u64_at(32);
    if l == 0 && o == 0 { None } else { Some(OffsetWithLength { offset: o, len: l }) }
  }

  /// For files: source map offset and length.
  pub fn file_source_map_offset(&self) -> Option<OffsetWithLength> {
    let o = self.read_u64_at(40);
    let l = self.read_u64_at(48);
    if l == 0 && o == 0 { None } else { Some(OffsetWithLength { offset: o, len: l }) }
  }

  /// For files: CJS export analysis offset and length.
  pub fn file_cjs_export_analysis_offset(&self) -> Option<OffsetWithLength> {
    let o = self.read_u64_at(56);
    let l = self.read_u64_at(64);
    if l == 0 && o == 0 { None } else { Some(OffsetWithLength { offset: o, len: l }) }
  }

  /// For files: mtime in milliseconds (None if 0).
  pub fn file_mtime(&self) -> Option<u128> {
    let ms = self.read_u64_at(72);
    if ms == 0 { None } else { Some(ms as u128) }
  }

  /// For symlinks: resolve destination from root path.
  pub fn symlink_resolve_dest_from_root(&self, root: &Path) -> PathBuf {
    debug_assert_eq!(self.kind(), BinaryVfsEntryKind::Symlink);
    let parts_start = u32::from_le_bytes(self.data[8..12].try_into().unwrap());
    let parts_count = u32::from_le_bytes(self.data[12..16].try_into().unwrap());
    let mut dest = root.to_path_buf();
    for i in 0..parts_count {
      let string_id = self.vfs.get_symlink_part_string_id(parts_start + i);
      dest.push(self.vfs.get_string(string_id));
    }
    dest
  }

  /// For symlinks: get the parts as a vector of string references.
  pub fn symlink_dest_parts(&self) -> Vec<&'static str> {
    debug_assert_eq!(self.kind(), BinaryVfsEntryKind::Symlink);
    let parts_start = u32::from_le_bytes(self.data[8..12].try_into().unwrap());
    let parts_count = u32::from_le_bytes(self.data[12..16].try_into().unwrap());
    (0..parts_count)
      .map(|i| {
        let string_id = self.vfs.get_symlink_part_string_id(parts_start + i);
        self.vfs.get_string(string_id)
      })
      .collect()
  }

  /// For symlinks: display the destination parts as a joined string.
  pub fn symlink_dest_display(&self) -> String {
    self.symlink_dest_parts().join("/")
  }
}

#[cfg(test)]
mod binary_vfs_tests {
  use super::*;

  #[test]
  fn test_roundtrip_binary_vfs() {
    // Build a VirtualDirectoryEntries tree
    let entries = VirtualDirectoryEntries::new(vec![
      VfsEntry::Dir(VirtualDirectory {
        name: "sub".to_string(),
        entries: VirtualDirectoryEntries::new(vec![
          VfsEntry::File(VirtualFile {
            name: "hello.txt".to_string(),
            offset: OffsetWithLength { offset: 0, len: 5 },
            is_valid_utf8: true,
            transpiled_offset: None,
            cjs_export_analysis_offset: None,
            source_map_offset: None,
            mtime: Some(12345),
          }),
          VfsEntry::Symlink(VirtualSymlink {
            name: "link.txt".to_string(),
            dest_parts: VirtualSymlinkParts::from_path(Path::new("sub/hello.txt")),
          }),
        ]),
      }),
      VfsEntry::File(VirtualFile {
        name: "root.txt".to_string(),
        offset: OffsetWithLength { offset: 5, len: 10 },
        is_valid_utf8: false,
        transpiled_offset: Some(OffsetWithLength { offset: 15, len: 20 }),
        cjs_export_analysis_offset: Some(OffsetWithLength { offset: 35, len: 8 }),
        source_map_offset: Some(OffsetWithLength { offset: 43, len: 12 }),
        mtime: None,
      }),
    ]);

    let serialized = serialize_vfs_binary(&entries);

    // Leak to get 'static lifetime for testing
    let static_data: &'static [u8] = Box::leak(serialized.into_boxed_slice());
    let (remaining, view) = BinaryVfsView::from_bytes(static_data).unwrap();
    assert!(remaining.is_empty());

    // Check root children
    let root = view.root_children();
    assert_eq!(root.len(), 2);

    // First child should be "root.txt" (sorted) or "sub" (sorted)
    // "root.txt" < "sub" alphabetically
    let first = root.get(0).unwrap();
    assert_eq!(first.name(), "root.txt");
    assert_eq!(first.kind(), BinaryVfsEntryKind::File);
    assert!(!first.file_is_valid_utf8());
    assert_eq!(first.file_data_offset().offset, 5);
    assert_eq!(first.file_data_offset().len, 10);
    assert_eq!(first.file_transpiled_offset().unwrap().offset, 15);
    assert_eq!(first.file_cjs_export_analysis_offset().unwrap().offset, 35);
    assert_eq!(first.file_source_map_offset().unwrap().offset, 43);
    assert!(first.file_mtime().is_none());

    let second = root.get(1).unwrap();
    assert_eq!(second.name(), "sub");
    assert_eq!(second.kind(), BinaryVfsEntryKind::Dir);

    let children = second.dir_children();
    assert_eq!(children.len(), 2);

    let hello = children.get(0).unwrap();
    assert_eq!(hello.name(), "hello.txt");
    assert_eq!(hello.kind(), BinaryVfsEntryKind::File);
    assert!(hello.file_is_valid_utf8());
    assert_eq!(hello.file_mtime(), Some(12345));

    let link = children.get(1).unwrap();
    assert_eq!(link.name(), "link.txt");
    assert_eq!(link.kind(), BinaryVfsEntryKind::Symlink);
    assert_eq!(link.symlink_dest_display(), "sub/hello.txt");

    // Test binary search
    let found = view.binary_search(
      view.root_children_start,
      view.root_children_count,
      "sub",
      FileSystemCaseSensitivity::Sensitive,
    );
    assert!(found.is_ok());
    let entry = view.get_entry(found.unwrap());
    assert_eq!(entry.name(), "sub");
  }
}
