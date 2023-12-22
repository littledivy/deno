// Copyright 2018-2023 the Deno authors. All rights reserved. MIT license.

use super::WebGpuResult;
use deno_core::error::AnyError;
use deno_core::op2;
use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use serde::Deserialize;
use std::borrow::Cow;
use std::ffi::c_void;
use std::rc::Rc;
use wgpu_types::SurfaceStatus;

deno_core::extension!(
  deno_webgpu_surface,
  deps = [deno_webidl, deno_web, deno_webgpu],
  ops = [
    op_webgpu_surface_configure,
    op_webgpu_surface_get_current_texture,
    op_webgpu_surface_present,
  ],
  esm = ["02_surface.js"],
);

pub struct WebGpuSurface(pub crate::Instance, pub wgpu_core::id::SurfaceId);
impl Resource for WebGpuSurface {
  fn name(&self) -> Cow<str> {
    "webGPUSurface".into()
  }

  fn close(self: Rc<Self>) {
    self.0.surface_drop(self.1);
  }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlphaMode {
  Opaque,
  Premultiplied,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceConfigureArgs {
  surface_rid: ResourceId,
  device_rid: ResourceId,
  format: wgpu_types::TextureFormat,
  usage: u32,
  width: u32,
  height: u32,
  present_mode: Option<wgpu_types::PresentMode>,
  alpha_mode: AlphaMode,
  view_formats: Vec<wgpu_types::TextureFormat>,
}

#[op2]
#[serde]
pub fn op_webgpu_surface_configure(
  state: &mut OpState,
  #[serde] args: SurfaceConfigureArgs,
) -> Result<WebGpuResult, AnyError> {
  let instance = state.borrow::<super::Instance>();
  let device_resource = state
    .resource_table
    .get::<super::WebGpuDevice>(args.device_rid)?;
  let device = device_resource.1;
  let surface_resource = state
    .resource_table
    .get::<WebGpuSurface>(args.surface_rid)?;
  let surface = surface_resource.1;

  let conf = wgpu_types::SurfaceConfiguration::<Vec<wgpu_types::TextureFormat>> {
    usage: wgpu_types::TextureUsages::from_bits_truncate(args.usage),
    format: args.format,
    width: args.width,
    height: args.height,
    present_mode: args.present_mode.unwrap_or_default(),
    alpha_mode: match args.alpha_mode {
      AlphaMode::Opaque => wgpu_types::CompositeAlphaMode::Opaque,
      AlphaMode::Premultiplied => wgpu_types::CompositeAlphaMode::PreMultiplied,
    },
    view_formats: args.view_formats,
  };

  let err =
    gfx_select!(device => instance.surface_configure(surface, device, &conf));

  Ok(WebGpuResult::maybe_err(err))
}

#[op2]
#[serde]
pub fn op_webgpu_surface_get_current_texture(
  state: &mut OpState,
  #[smi] device_rid: ResourceId,
  #[smi] surface_rid: ResourceId,
) -> Result<WebGpuResult, AnyError> {
  let instance = state.borrow::<super::Instance>();
  let device_resource = state
    .resource_table
    .get::<super::WebGpuDevice>(device_rid)?;
  let device = device_resource.1;
  let surface_resource =
    state.resource_table.get::<WebGpuSurface>(surface_rid)?;
  let surface = surface_resource.1;

  let output =
    gfx_select!(device => instance.surface_get_current_texture(surface, ()))?;

  match output.status {
    SurfaceStatus::Good | SurfaceStatus::Suboptimal => {
      let id = output.texture_id.unwrap();
      let rid = state.resource_table.add(crate::texture::WebGpuTexture {
        instance: instance.clone(),
        id,
        owned: false,
      });
      Ok(WebGpuResult::rid(rid))
    }
    _ => Err(AnyError::msg("Invalid Surface Status")),
  }
}

#[op2(fast)]
pub fn op_webgpu_surface_present(
  state: &mut OpState,
  #[smi] device_rid: ResourceId,
  #[smi] surface_rid: ResourceId,
) -> Result<(), AnyError> {
  let instance = state.borrow::<super::Instance>();
  let device_resource = state
    .resource_table
    .get::<super::WebGpuDevice>(device_rid)?;
  let device = device_resource.1;
  let surface_resource =
    state.resource_table.get::<WebGpuSurface>(surface_rid)?;
  let surface = surface_resource.1;

  let _ = gfx_select!(device => instance.surface_present(surface))?;

  Ok(())
}

#[op2(fast)]
#[smi]
pub fn op_webgpu_surface_create(
  state: &mut OpState,
  #[string] system: &str,
  win_handle: *const c_void,
  display_handle: *const c_void,
) -> Result<ResourceId, AnyError> {
  let instance = state.borrow::<super::Instance>();

  let (win_handle, display_handle) = raw_window(win_handle, display_handle);
  let surface = {
    instance.instance_create_surface(
      display_handle,
      win_handle,
      Default::default(),
    )
  };

  let rid = state
    .resource_table
    .add(WebGpuSurface(instance.clone(), surface));
  Ok(rid)
}

#[cfg(target_os = "macos")]
fn raw_window(
  ns_window: *const c_void,
  ns_view: *const c_void,
) -> (raw_window_handle::RawWindowHandle, raw_window_handle::RawDisplayHandle) {
  let win_handle = {
    let mut handle = raw_window_handle::AppKitWindowHandle::empty();
    handle.ns_window = ns_window as *mut c_void;
    handle.ns_view = ns_view as *mut c_void;

    raw_window_handle::RawWindowHandle::AppKit(handle)
  };
  let display_handle =
    raw_window_handle::RawDisplayHandle::AppKit(raw_window_handle::AppKitDisplayHandle::empty());
  (win_handle, display_handle)
}

#[cfg(target_os = "windows")]
fn raw_window(
  core_window: *const c_void,
  _: *const c_void,
) -> (raw_window_handle::RawWindowHandle, raw_window_handle::RawDisplayHandle) {
  use raw_window_handle::WinRtWindowHandle;
  use raw_window_handle::WindowsDisplayHandle;

  let mut handle = WinRtWindowHandle::empty();
  handle.core_window = core_window as *mut c_void;

  let win_handle = raw_window_handle::RawWindowHandle::WinRt(handle);
  let display_handle = raw_window_handle::RawDisplayHandle::Windows(WindowsDisplayHandle::empty());
  (win_handle, display_handle)
}