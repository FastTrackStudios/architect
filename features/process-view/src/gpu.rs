//! Frames on the GPU, shared between two processes' wgpu devices on one
//! adapter (Linux, Vulkan): the guest allocates a texture whose memory
//! is exportable as a dma-buf ([`export_texture`]), the host imports
//! the dma-buf as a texture of its own device ([`import_dmabuf`]). Both
//! see the same memory: no copy between them.
//!
//! The image is `R8G8B8A8_UNORM` in a linear layout (DRM format
//! modifier 0), the one every driver that exports dma-bufs can import.
//! wgpu (30) has the import (`wgpu_hal`'s `texture_from_dmabuf_fd`) but
//! no export, so the export is written here with `ash` on the device
//! wgpu made.
//!
//! **Synchronisation.** The guest waits for its GPU to finish a frame
//! before it tells the host the frame is ready, and draws only into
//! slots the host has released; the host releases a slot one frame
//! after it stopped showing it. That is CPU-side, implicit-sync-free
//! and correct on every driver, at the cost of the guest's wait. A
//! `sync_file` exported from a `VkSemaphore` (`VK_KHR_external_semaphore_fd`)
//! would let the host's GPU wait instead: a follow-up.

use std::os::fd::{FromRawFd as _, OwnedFd};

use ash::vk;
use wgpu::hal;

use crate::wire::Adapter;

/// What went wrong sharing a texture.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("the device is not a Vulkan one")]
    NotVulkan,
    #[error("the device cannot share dma-bufs (VULKAN_EXTERNAL_MEMORY_DMA_BUF is off)")]
    NoDmabuf,
    #[error("no memory type fits the image")]
    NoMemoryType,
    #[error("Vulkan: {0}")]
    Vulkan(vk::Result),
    #[error("wgpu-hal: {0}")]
    Hal(#[from] hal::DeviceError),
    #[error("a size out of range")]
    Range,
}

impl From<vk::Result> for GpuError {
    fn from(e: vk::Result) -> Self {
        Self::Vulkan(e)
    }
}

/// The frames' format.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The linear DRM format modifier.
pub const LINEAR: u64 = 0;

/// `adapter` as the wire names it.
#[must_use]
pub fn adapter_info(adapter: &wgpu::Adapter) -> Adapter {
    let info = adapter.get_info();
    Adapter {
        vendor: info.vendor,
        device: info.device,
        name: info.name,
    }
}

/// Whether `device` can import (and export) dma-buf textures.
#[must_use]
pub fn can_share(device: &wgpu::Device) -> bool {
    device
        .features()
        .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
        // SAFETY: only asked whether the device is a Vulkan one.
        && unsafe { device.as_hal::<hal::api::Vulkan>() }.is_some()
}

fn descriptor(
    width: u32,
    height: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("process view frame"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage,
        view_formats: &[],
    }
}

fn hal_descriptor(width: u32, height: u32) -> hal::TextureDescriptor<'static> {
    hal::TextureDescriptor {
        label: Some("process view frame"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUses::COPY_SRC
            | wgpu::TextureUses::COPY_DST
            | wgpu::TextureUses::RESOURCE,
        memory_flags: hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    }
}

/// The usages both sides give a shared frame: the guest copies into
/// it, the host's renderer copies out of it or samples it.
const USAGE: wgpu::TextureUsages = wgpu::TextureUsages::COPY_SRC
    .union(wgpu::TextureUsages::COPY_DST)
    .union(wgpu::TextureUsages::TEXTURE_BINDING);

/// A texture of the guest's device whose memory is a dma-buf.
#[derive(Debug)]
pub struct Exported {
    pub texture: wgpu::Texture,
    /// The dma-buf: sent to the host (a duplicate each time).
    pub fd: OwnedFd,
    pub stride: u32,
    pub offset: u32,
    pub modifier: u64,
}

/// A `width` × `height` frame on `device` whose memory the host can
/// import.
pub fn export_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<Exported, GpuError> {
    if !device
        .features()
        .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
    {
        return Err(GpuError::NoDmabuf);
    }
    // SAFETY: the raw handles are used only while `hal_device` lives, and
    // every object made with them is handed to wgpu (which owns it from
    // then on) or destroyed here on failure.
    let hal_device = unsafe { device.as_hal::<hal::api::Vulkan>() }.ok_or(GpuError::NotVulkan)?;
    let raw = hal_device.raw_device().clone();
    let instance = hal_device.shared_instance().raw_instance();
    let physical = hal_device.raw_physical_device();
    let memory_fd = ash::khr::external_memory_fd::Device::new(instance, &raw);
    let modifiers = ash::ext::image_drm_format_modifier::Device::new(instance, &raw);

    let list = [LINEAR];
    let mut modifier_list =
        vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&list);
    let mut external = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(
            vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::SAMPLED,
        )
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external)
        .push_next(&mut modifier_list);
    // SAFETY: a valid create info on this device.
    let image = unsafe { raw.create_image(&info, None) }?;
    let destroy_image = move |raw: &ash::Device| unsafe { raw.destroy_image(image, None) };

    // SAFETY: `image` was just made on `raw`.
    let requirements = unsafe { raw.get_image_memory_requirements(image) };
    // SAFETY: `physical` is the device's own.
    let properties = unsafe { instance.get_physical_device_memory_properties(physical) };
    let Some(type_index) = memory_type(&properties, requirements.memory_type_bits) else {
        destroy_image(&raw);
        return Err(GpuError::NoMemoryType);
    };
    let mut export = vk::ExportMemoryAllocateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let allocate = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(type_index)
        .push_next(&mut export)
        .push_next(&mut dedicated);
    // SAFETY: a valid allocation for `image`.
    let memory = match unsafe { raw.allocate_memory(&allocate, None) } {
        Ok(m) => m,
        Err(e) => {
            destroy_image(&raw);
            return Err(e.into());
        }
    };
    let free = move |raw: &ash::Device| unsafe {
        raw.free_memory(memory, None);
        raw.destroy_image(image, None);
    };
    // SAFETY: `memory` was allocated for `image`.
    if let Err(e) = unsafe { raw.bind_image_memory(image, memory, 0) } {
        free(&raw);
        return Err(e.into());
    }

    let get_fd = vk::MemoryGetFdInfoKHR::default()
        .memory(memory)
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    // SAFETY: `memory` was allocated exportable as a dma-buf.
    let fd = match unsafe { memory_fd.get_memory_fd(&get_fd) } {
        // SAFETY: vkGetMemoryFdKHR hands us a new descriptor we own.
        Ok(fd) => unsafe { OwnedFd::from_raw_fd(fd) },
        Err(e) => {
            free(&raw);
            return Err(e.into());
        }
    };
    let mut chosen = vk::ImageDrmFormatModifierPropertiesEXT::default();
    // SAFETY: `image` was made with a DRM format modifier tiling.
    if let Err(e) =
        unsafe { modifiers.get_image_drm_format_modifier_properties(image, &mut chosen) }
    {
        free(&raw);
        return Err(e.into());
    }
    // SAFETY: plane 0 of a single-plane modifier image.
    let layout = unsafe {
        raw.get_image_subresource_layout(
            image,
            vk::ImageSubresource {
                aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
                mip_level: 0,
                array_layer: 0,
            },
        )
    };
    let (Ok(stride), Ok(offset)) = (
        u32::try_from(layout.row_pitch),
        u32::try_from(layout.offset),
    ) else {
        free(&raw);
        return Err(GpuError::Range);
    };

    let owner = raw.clone();
    let drop_callback: hal::DropCallback = Box::new(move || free(&owner));
    // SAFETY: `image` was made respecting the descriptor (format,
    // extent, usages); the callback frees it and its memory once wgpu is
    // done with it.
    let hal_texture = unsafe {
        hal_device.texture_from_raw(
            image,
            &hal_descriptor(width, height),
            Some(drop_callback),
            hal::vulkan::TextureMemory::External,
        )
    };
    drop(hal_device);
    // SAFETY: made on this device, respecting the descriptor; its
    // contents are undefined until the first copy into it.
    let texture = unsafe {
        device.create_texture_from_hal::<hal::api::Vulkan>(
            hal_texture,
            &descriptor(width, height, USAGE),
            wgpu::TextureUses::UNINITIALIZED,
        )
    };
    Ok(Exported {
        texture,
        fd,
        stride,
        offset,
        modifier: chosen.drm_format_modifier,
    })
}

/// The first device-local memory type in `bits`, else any in it.
fn memory_type(properties: &vk::PhysicalDeviceMemoryProperties, bits: u32) -> Option<u32> {
    let count = usize::try_from(properties.memory_type_count).ok()?;
    let types = properties.memory_types.get(..count)?;
    let fits = |i: usize| {
        u32::try_from(i)
            .ok()
            .filter(|i| bits & 1u32.checked_shl(*i).unwrap_or(0) != 0)
    };
    types
        .iter()
        .enumerate()
        .find(|(i, t)| {
            fits(*i).is_some()
                && t.property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .or_else(|| types.iter().enumerate().find(|(i, _)| fits(*i).is_some()))
        .and_then(|(i, _)| u32::try_from(i).ok())
}

/// The guest's dma-buf `fd` (a frame of `width` × `height`, laid out as
/// `stride`, `offset` and `modifier` say) as a texture of the host's
/// `device`. Vulkan takes `fd`, on failure too.
pub fn import_dmabuf(
    device: &wgpu::Device,
    fd: OwnedFd,
    (width, height): (u32, u32),
    (stride, offset, modifier): (u32, u32, u64),
) -> Result<wgpu::Texture, GpuError> {
    if !can_share(device) {
        return Err(GpuError::NoDmabuf);
    }
    // SAFETY: as in `export_texture`; the hal texture is handed to wgpu.
    let hal_device = unsafe { device.as_hal::<hal::api::Vulkan>() }.ok_or(GpuError::NotVulkan)?;
    // SAFETY: `fd` is a dma-buf of this layout, from the guest's export.
    let hal_texture = unsafe {
        hal_device.texture_from_dmabuf_fd(
            fd,
            &hal_descriptor(width, height),
            modifier,
            u64::from(stride),
            u64::from(offset),
        )
    }?;
    drop(hal_device);
    // SAFETY: made on this device, respecting the descriptor.
    Ok(unsafe {
        device.create_texture_from_hal::<hal::api::Vulkan>(
            hal_texture,
            &descriptor(width, height, USAGE),
            wgpu::TextureUses::UNINITIALIZED,
        )
    })
}

/// A texture of `width` × `height` the host uploads shared-memory
/// frames into.
#[must_use]
pub fn upload_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&descriptor(width, height, USAGE))
}
