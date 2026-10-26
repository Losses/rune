//! Linux: render straight into Vulkan images whose memory Flutter's OpenGL context imports, so the
//! app hands frames to Flutter without copying a pixel.
//!
//! Each target is an `R8G8B8A8_UNORM` image with optimal tiling in a dedicated, device-local
//! allocation that can be exported as an opaque file descriptor (`VK_KHR_external_memory_fd`). The
//! app imports that memory once per image into OpenGL (`GL_EXT_memory_object_fd`) and makes a
//! texture on it. OpenGL can only import memory of its own GPU and driver, so the visualizer must
//! render on the Vulkan device whose UUIDs match Flutter's context: see [`adapter_with_uuids`].
//! As `GL_EXT_external_objects` asks of images that OpenGL aliases (its issues 13 and 17), the
//! images are created with every usage of the format that OpenGL could put a texture to, and
//! with a mutable format.
//!
//! Handing a frame over follows the Vulkan rules for external memory: after drawing, a barrier
//! releases the image to `VK_QUEUE_FAMILY_EXTERNAL` in `SHADER_READ_ONLY_OPTIMAL`, and the
//! submission signals a semaphore made for that frame alone and exported as an opaque file
//! descriptor (`VK_KHR_external_semaphore_fd`). The app waits for it with `glWaitSemaphoreEXT`,
//! naming the texture and `GL_LAYOUT_SHADER_READ_ONLY_EXT`, which makes the writes visible to
//! OpenGL and tells it the layout (`GL_EXT_semaphore_fd`). A semaphore per frame is signalled once
//! and waited for at most once, as binary semaphores require, whether or not Flutter shows the
//! frame. wgpu has no queue family transfers, so the barriers are recorded with the raw device in
//! command buffers of their own, submitted with wgpu's in one batch.
//!
//! The way back is not synchronised: OpenGL never signals that it has finished sampling. As on
//! the other platforms, an image is drawn into again only once no frame holds it and a few newer
//! frames were published. It is then acquired from the external queue family with an undefined
//! old layout, which is valid whatever OpenGL did with it, since every frame is drawn from a clear.
//! wgpu does not see these barriers: it still believes the image is a render target after the
//! last frame, which is the layout the acquire leaves it in, so its own barriers stay correct.

use std::collections::VecDeque;
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use ash::{khr, vk};
use wgpu::hal;

use crate::DeviceUuids;

/// Published frames that stay referenced here after a newer one replaced them, so that an image
/// is drawn into again only a few frames after Flutter let go of it: its compositor may still
/// sample the last frame it was given while the next one is being drawn.
const KEEP_RECENT: usize = 3;
/// Upper bound on the images of one size: the recent ones (the latest among them), the ones the GPU
/// is drawing (`crate::in_flight`), finished ones not handed out yet, and Flutter's.
const MAX_IMAGES: usize = 8;

/// The Vulkan format of [`crate::pipeline::FORMAT`], which OpenGL imports as `GL_RGBA8`.
const VK_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
/// The layout an image is handed to OpenGL in; the app waits for a frame with the matching
/// `GL_LAYOUT_SHADER_READ_ONLY_EXT`.
const SHARED_LAYOUT: vk::ImageLayout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
const MEMORY_HANDLE: vk::ExternalMemoryHandleTypeFlags =
    vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD;
const SEMAPHORE_HANDLE: vk::ExternalSemaphoreHandleTypeFlags =
    vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD;

/// Ids of images and of sets of images, never reused in the process, so that the app can tell
/// them apart across sizes and restarts.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The Vulkan adapter whose device and driver UUIDs are `uuids`, if there is one: the GPU and
/// driver Flutter's OpenGL context runs on, the only ones whose memory it can import.
pub fn adapter_with_uuids(instance: &wgpu::Instance, uuids: &DeviceUuids) -> Option<wgpu::Adapter> {
    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
        .into_iter()
        .find(|adapter| {
            // SAFETY: only reads the adapter's properties; the guard is dropped before wgpu
            // uses the adapter again.
            let Some(hal_adapter) = (unsafe { adapter.as_hal::<hal::api::Vulkan>() }) else {
                return false;
            };
            adapter_uuids(&hal_adapter).as_ref() == Some(uuids)
        })
}

/// The device and driver UUIDs of an adapter (`VkPhysicalDeviceIDProperties`), if the instance
/// and the device have Vulkan 1.1, which sharing needs anyway.
fn adapter_uuids(adapter: &hal::vulkan::Adapter) -> Option<DeviceUuids> {
    let instance = adapter.shared_instance();
    if instance.instance_api_version() < vk::API_VERSION_1_1
        || adapter
            .physical_device_capabilities()
            .properties()
            .api_version
            < vk::API_VERSION_1_1
    {
        return None;
    }
    let mut ids = vk::PhysicalDeviceIDProperties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut ids);
    // SAFETY: a Vulkan 1.1 query, which the instance and the device have (checked above), on a
    // valid physical device with a valid chain.
    unsafe {
        instance
            .raw_instance()
            .get_physical_device_properties2(adapter.raw_physical_device(), &mut properties);
    }
    Some(DeviceUuids {
        device: ids.device_uuid,
        driver: ids.driver_uuid,
    })
}

/// A device on `adapter` that can also export semaphores, which [`SharedTargets`] needs and wgpu
/// does not enable by itself. Otherwise like `request_device` with the default descriptor. Fails
/// if the adapter cannot share images and semaphores.
pub fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue)> {
    let desc = wgpu::DeviceDescriptor {
        // For the GPU time of a frame (see `crate::gpu_time`), where there are timestamps.
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    };
    let open = {
        // SAFETY: the guard is dropped before wgpu uses the adapter again.
        let hal_adapter = unsafe { adapter.as_hal::<hal::api::Vulkan>() }
            .context("The GPU adapter does not run on Vulkan")?;
        let capabilities = hal_adapter.physical_device_capabilities();
        if capabilities.properties().api_version < vk::API_VERSION_1_1 {
            bail!("The GPU adapter does not have Vulkan 1.1");
        }
        for extension in [
            khr::external_memory_fd::NAME,
            khr::external_semaphore_fd::NAME,
        ] {
            if !capabilities.supports_extension(extension) {
                bail!("The GPU adapter lacks {}", extension.to_string_lossy());
            }
        }
        // SAFETY: the features, limits and memory hints are the descriptor's, as wgpu's own
        // `request_device` passes them; the callback only adds an extension the adapter supports
        // (checked above), whose dependency `VK_KHR_external_semaphore` is part of Vulkan 1.1.
        unsafe {
            hal_adapter.open_with_callback(
                desc.required_features,
                &desc.required_limits,
                &desc.memory_hints,
                Some(Box::new(
                    |args: hal::vulkan::CreateDeviceCallbackArgs<'_, '_, '_>| {
                        args.extensions.push(khr::external_semaphore_fd::NAME);
                    },
                )),
            )
        }
        .context("Failed to create the GPU device")?
    };
    // SAFETY: `open` comes from this adapter, with exactly the features `desc` asks for.
    unsafe { adapter.create_device_from_hal(open, &desc) }
        .context("Failed to create the GPU device")
}

/// What the images and frames need of the Vulkan device. It does not keep the device alive:
/// whatever uses it also holds a texture view, which does.
struct Vulkan {
    device: ash::Device,
    memory_fd: khr::external_memory_fd::Device,
    semaphore_fd: khr::external_semaphore_fd::Device,
    /// The family of wgpu's queue, which the images are released from and acquired to.
    queue_family: u32,
}

struct Image {
    id: u64,
    /// The id of the [`SharedTargets`] that made it.
    set: u64,
    raw: vk::Image,
    /// Owned by wgpu, which frees it with the texture.
    memory: vk::DeviceMemory,
    memory_size: u64,
    /// Keeps the texture, its image and memory, and the device alive.
    view: wgpu::TextureView,
    vulkan: Arc<Vulkan>,
}

/// A render target whose memory can be exported. Clones refer to the same image; it is free to be
/// drawn into again once only the pool refers to it.
#[derive(Clone)]
struct SharedImage(Arc<Image>);

impl SharedImage {
    /// Whether nothing but the pool refers to the image. Only the pool hands out new references,
    /// so a free image stays free until it does.
    fn is_free(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }
}

/// A semaphore that one frame's submission signals.
struct Semaphore {
    raw: vk::Semaphore,
    vulkan: Arc<Vulkan>,
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        // SAFETY: the semaphore belongs to this device, which is alive (see `Vulkan`). Any
        // submission that signals it has finished: the render thread hands a frame out or drops
        // it only once the GPU has finished it (see `crate::in_flight`), and leaks the frames on
        // their way if waiting for them fails. Nothing in Vulkan waits for it, and OpenGL holds
        // its own reference to the payload once it imported it.
        unsafe { self.vulkan.device.destroy_semaphore(self.raw, None) };
    }
}

/// A frame drawn into a shared image and handed over to OpenGL.
pub struct SharedFrame {
    // Before `image`, whose view keeps the device alive while the semaphore is destroyed.
    semaphore: Semaphore,
    image: SharedImage,
}

impl SharedFrame {
    /// The image the frame is in. Frames in the same image have the same id; ids are never
    /// reused.
    pub fn image_id(&self) -> u64 {
        self.image.0.id
    }

    /// The set of images the frame's image belongs to: one per size and start. The visualizer no
    /// longer draws into images of an earlier set.
    pub fn image_set(&self) -> u64 {
        self.image.0.set
    }

    /// The size in bytes of the image's dedicated allocation, as `glImportMemoryFdEXT` wants it.
    pub fn memory_size(&self) -> u64 {
        self.image.0.memory_size
    }

    /// A new file descriptor for the image's memory (`VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD`),
    /// owned by the caller.
    pub fn export_memory(&self) -> Result<OwnedFd> {
        let image = &self.image.0;
        let info = vk::MemoryGetFdInfoKHR::default()
            .memory(image.memory)
            .handle_type(MEMORY_HANDLE);
        // SAFETY: the memory was allocated exportable as an opaque fd and stays allocated while
        // this frame holds the image, whose view keeps the texture that owns it.
        let fd = unsafe { image.vulkan.memory_fd.get_memory_fd(&info) }
            .context("Failed to export the memory of a shared image")?;
        // SAFETY: a new file descriptor that nothing else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// A new file descriptor for the semaphore that the frame's submission signalled
    /// (`VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_FD`), owned by the caller. Wait for it once,
    /// before sampling the frame, with the image's layout `GL_LAYOUT_SHADER_READ_ONLY_EXT`.
    pub fn export_semaphore(&self) -> Result<OwnedFd> {
        let semaphore = &self.semaphore;
        let info = vk::SemaphoreGetFdInfoKHR::default()
            .semaphore(semaphore.raw)
            .handle_type(SEMAPHORE_HANDLE);
        // SAFETY: the semaphore was created exportable as an opaque fd, which has reference
        // semantics, and lives as long as this frame.
        let fd = unsafe { semaphore.vulkan.semaphore_fd.get_semaphore_fd(&info) }
            .context("Failed to export the semaphore of a shared frame")?;
        // SAFETY: a new file descriptor that nothing else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

/// The image a frame is drawn into next, from [`SharedTargets::next`].
pub struct NextTarget {
    index: usize,
    semaphore: Semaphore,
}

struct Slot {
    image: SharedImage,
    /// Whether a frame was drawn into the image, which then belongs to the external queue family
    /// until it is acquired back.
    drawn: bool,
}

/// The render targets of one size.
pub struct SharedTargets {
    width: u32,
    height: u32,
    /// The id of this set of images.
    set: u64,
    vulkan: Arc<Vulkan>,
    usage: vk::ImageUsageFlags,
    /// The properties of the device's memory types, by index.
    memory_types: Vec<vk::MemoryPropertyFlags>,
    images: Vec<Slot>,
    recent: VecDeque<SharedImage>,
}

impl SharedTargets {
    /// Creates the first image at once, so that a device that cannot share images fails here,
    /// before anything is drawn.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Result<Self> {
        let (vulkan, usage, memory_types) = {
            // SAFETY: the guard is dropped before wgpu is used again.
            let hal_device = unsafe { device.as_hal::<hal::api::Vulkan>() }
                .context("The GPU device does not run on Vulkan")?;
            for extension in [
                khr::external_memory_fd::NAME,
                khr::external_semaphore_fd::NAME,
            ] {
                if !hal_device.enabled_device_extensions().contains(&extension) {
                    bail!("The GPU device lacks {}", extension.to_string_lossy());
                }
            }
            let instance = hal_device.shared_instance();
            let raw_instance = instance.raw_instance();
            let physical = hal_device.raw_physical_device();
            // SAFETY: a plain query on a valid physical device.
            let device_version =
                unsafe { raw_instance.get_physical_device_properties(physical) }.api_version;
            if instance.instance_api_version() < vk::API_VERSION_1_1
                || device_version < vk::API_VERSION_1_1
            {
                bail!("The GPU device does not have Vulkan 1.1");
            }
            let usage = image_usage(raw_instance, physical, width, height)?;
            check_semaphore_export(raw_instance, physical)?;
            let raw = hal_device.raw_device();
            let vulkan = Vulkan {
                device: raw.clone(),
                memory_fd: khr::external_memory_fd::Device::new(raw_instance, raw),
                semaphore_fd: khr::external_semaphore_fd::Device::new(raw_instance, raw),
                queue_family: hal_device.queue_family_index(),
            };
            // SAFETY: a plain query on a valid physical device.
            let memory_types =
                unsafe { raw_instance.get_physical_device_memory_properties(physical) }
                    .memory_types_as_slice()
                    .iter()
                    .map(|memory_type| memory_type.property_flags)
                    .collect();
            (Arc::new(vulkan), usage, memory_types)
        };
        let mut targets = Self {
            width,
            height,
            set: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            vulkan,
            usage,
            memory_types,
            images: Vec::new(),
            recent: VecDeque::new(),
        };
        let first = targets.create(device)?;
        targets.images.push(Slot {
            image: first,
            drawn: false,
        });
        Ok(targets)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// A free image and a semaphore for the frame, or `None` while every image is still in use
    /// and the pool is full (the frame is skipped).
    pub fn next(&mut self, device: &wgpu::Device) -> Result<Option<NextTarget>> {
        let index = match self.images.iter().position(|slot| slot.image.is_free()) {
            Some(index) => index,
            None if self.images.len() < MAX_IMAGES => {
                let image = self.create(device)?;
                self.images.push(Slot {
                    image,
                    drawn: false,
                });
                self.images.len() - 1
            }
            None => return Ok(None),
        };
        let mut export = vk::ExportSemaphoreCreateInfo::default().handle_types(SEMAPHORE_HANDLE);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
        // SAFETY: a valid create info on this device, whose semaphores can be exported as
        // opaque fds (checked in `new`).
        let raw = unsafe { self.vulkan.device.create_semaphore(&info, None) }
            .context("Failed to create a semaphore to share")?;
        Ok(Some(NextTarget {
            index,
            semaphore: Semaphore {
                raw,
                vulkan: Arc::clone(&self.vulkan),
            },
        }))
    }

    /// Draw a frame with `draw` into the image from [`Self::next`] and hand it over to OpenGL,
    /// without waiting for the GPU. The frame must not be published or dropped before the GPU has
    /// finished the submission that draws it (the last one on `queue`), which signals its
    /// semaphore. `None` if wgpu refused the submission, which then never signals the semaphore
    /// (the frame is skipped).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        next: NextTarget,
        draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
    ) -> Result<Option<SharedFrame>> {
        let slot = &self.images[next.index];
        let image = slot.image.clone();
        let vulkan = &self.vulkan;
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        // Take the image back from OpenGL, discarding what is in it (every frame starts with a
        // clear). Before its first frame it belongs to no queue family yet.
        let acquire = if slot.drawn {
            Some(barrier(
                device,
                &vulkan.device,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::ImageMemoryBarrier::default()
                    .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                    .dst_queue_family_index(vulkan.queue_family)
                    .image(image.0.raw)
                    .subresource_range(range),
            )?)
        } else {
            None
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        draw(&mut encoder, &image.0.view);
        // Hand it to OpenGL in the layout the app waits with. wgpu leaves a render target in
        // `COLOR_ATTACHMENT_OPTIMAL`.
        let release = barrier(
            device,
            &vulkan.device,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(SHARED_LAYOUT)
                .src_queue_family_index(vulkan.queue_family)
                .dst_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                .image(image.0.raw)
                .subresource_range(range),
        )?;

        {
            // SAFETY: the guard is dropped before wgpu is used again.
            let hal_queue = unsafe { queue.as_hal::<hal::api::Vulkan>() }
                .context("The GPU queue does not run on Vulkan")?;
            // The next submission signals it, after all of its command buffers.
            hal_queue.add_signal_semaphore(next.semaphore.raw, None);
        }
        queue.submit(acquire.into_iter().chain([encoder.finish(), release]));
        let submitted = {
            // SAFETY: as above.
            let hal_queue = unsafe { queue.as_hal::<hal::api::Vulkan>() }
                .context("The GPU queue does not run on Vulkan")?;
            // Still pending if wgpu refused the submission; it must not go to a later one.
            !hal_queue.remove_signal_semaphore(next.semaphore.raw)
        };
        if !submitted {
            return Ok(None);
        }
        // The release is on its way: the next frame in this image acquires it back, after it on
        // the same queue.
        self.images[next.index].drawn = true;
        Ok(Some(SharedFrame {
            semaphore: next.semaphore,
            image,
        }))
    }

    /// Remember a frame that was just published (see [`KEEP_RECENT`]).
    pub fn keep(&mut self, frame: &SharedFrame) {
        self.recent.push_back(frame.image.clone());
        while self.recent.len() > KEEP_RECENT {
            self.recent.pop_front();
        }
    }

    fn create(&self, device: &wgpu::Device) -> Result<SharedImage> {
        let raw_device = &self.vulkan.device;
        let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(MEMORY_HANDLE);
        let info = vk::ImageCreateInfo::default()
            .flags(vk::ImageCreateFlags::MUTABLE_FORMAT)
            .image_type(vk::ImageType::TYPE_2D)
            .format(VK_FORMAT)
            .extent(vk::Extent3D {
                width: self.width,
                height: self.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(self.usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut external);
        // SAFETY: a valid create info on this device; `new` checked that it can export images
        // of this format, usage and size.
        let raw = unsafe { raw_device.create_image(&info, None) }
            .context("Failed to create a shared image")?;
        let (memory, memory_size) = match self.allocate(raw) {
            Ok(memory) => memory,
            Err(e) => {
                // SAFETY: the image was just created and nothing uses it.
                unsafe { raw_device.destroy_image(raw, None) };
                return Err(e);
            }
        };
        let size = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        let hal_texture = {
            // SAFETY: the guard is dropped before wgpu is used again.
            let Some(hal_device) = (unsafe { device.as_hal::<hal::api::Vulkan>() }) else {
                // SAFETY: both were just created and nothing uses them.
                unsafe {
                    raw_device.destroy_image(raw, None);
                    raw_device.free_memory(memory, None);
                }
                bail!("The GPU device does not run on Vulkan");
            };
            // SAFETY: `raw` is a 2D image of this device of `size`, in the format of `FORMAT`,
            // with one layer, one mip level and one sample, and usable as a render target and
            // sampled (among others), as described below. Without a drop callback and with
            // dedicated memory, wgpu takes over the image and its memory and frees both once the
            // texture is gone and the GPU has finished with it.
            unsafe {
                hal_device.texture_from_raw(
                    raw,
                    &hal::TextureDescriptor {
                        label: Some("visualizer shared target"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: crate::pipeline::FORMAT,
                        usage: wgpu::TextureUses::COLOR_TARGET | wgpu::TextureUses::RESOURCE,
                        memory_flags: hal::MemoryFlags::empty(),
                        view_formats: Vec::new(),
                    },
                    None,
                    hal::vulkan::TextureMemory::Dedicated(memory),
                )
            }
        };
        // SAFETY: `hal_texture` was made on this device from an image matching the descriptor
        // below, in the undefined layout that `UNINITIALIZED` stands for.
        let texture = unsafe {
            device.create_texture_from_hal::<hal::api::Vulkan>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("visualizer shared target"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: crate::pipeline::FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::TextureUses::UNINITIALIZED,
            )
        };
        Ok(SharedImage(Arc::new(Image {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            set: self.set,
            raw,
            memory,
            memory_size,
            view: texture.create_view(&Default::default()),
            vulkan: Arc::clone(&self.vulkan),
        })))
    }

    /// Dedicated, exportable device memory bound to `image`, and its size.
    fn allocate(&self, image: vk::Image) -> Result<(vk::DeviceMemory, u64)> {
        let raw_device = &self.vulkan.device;
        // SAFETY: a valid image of this device.
        let requirements = unsafe { raw_device.get_image_memory_requirements(image) };
        let memory_type = self
            .memory_types
            .iter()
            .enumerate()
            .position(|(index, flags)| {
                requirements.memory_type_bits & (1 << index) != 0
                    && flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .context("No device memory fits a shared image")?;
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let mut export = vk::ExportMemoryAllocateInfo::default().handle_types(MEMORY_HANDLE);
        let info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type as u32)
            .push_next(&mut export)
            .push_next(&mut dedicated);
        // SAFETY: a valid allocate info for `image` on this device (Vulkan 1.1, checked in
        // `new`, has dedicated allocations).
        let memory = unsafe { raw_device.allocate_memory(&info, None) }
            .context("Failed to allocate the memory of a shared image")?;
        // SAFETY: the memory was allocated for this image, which has none bound yet.
        if let Err(e) = unsafe { raw_device.bind_image_memory(image, memory, 0) } {
            // SAFETY: the memory was just allocated and nothing uses it.
            unsafe { raw_device.free_memory(memory, None) };
            return Err(e).context("Failed to bind the memory of a shared image");
        }
        Ok((memory, requirements.size))
    }
}

/// The usage of the shared images: every usage of the format with optimal tiling that OpenGL could
/// put a texture to, as `GL_EXT_external_objects` asks of images it aliases. Fails unless such
/// images can be rendered to, sampled and exported at `width` x `height`.
fn image_usage(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    width: u32,
    height: u32,
) -> Result<vk::ImageUsageFlags> {
    // SAFETY: a plain query on a valid physical device.
    let features = unsafe { instance.get_physical_device_format_properties(physical, VK_FORMAT) }
        .optimal_tiling_features;
    let usage = [
        (
            vk::FormatFeatureFlags::TRANSFER_SRC,
            vk::ImageUsageFlags::TRANSFER_SRC,
        ),
        (
            vk::FormatFeatureFlags::TRANSFER_DST,
            vk::ImageUsageFlags::TRANSFER_DST,
        ),
        (
            vk::FormatFeatureFlags::SAMPLED_IMAGE,
            vk::ImageUsageFlags::SAMPLED,
        ),
        (
            vk::FormatFeatureFlags::STORAGE_IMAGE,
            vk::ImageUsageFlags::STORAGE,
        ),
        (
            vk::FormatFeatureFlags::COLOR_ATTACHMENT,
            vk::ImageUsageFlags::COLOR_ATTACHMENT,
        ),
    ]
    .into_iter()
    .filter(|(feature, _)| features.contains(*feature))
    .fold(vk::ImageUsageFlags::empty(), |usage, (_, flag)| {
        usage | flag
    });
    if !usage.contains(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::COLOR_ATTACHMENT) {
        bail!("The GPU cannot render to and sample RGBA8 images");
    }
    let mut external_info =
        vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(MEMORY_HANDLE);
    let info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(VK_FORMAT)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage)
        .flags(vk::ImageCreateFlags::MUTABLE_FORMAT)
        .push_next(&mut external_info);
    let mut external = vk::ExternalImageFormatProperties::default();
    let mut properties = vk::ImageFormatProperties2::default().push_next(&mut external);
    // SAFETY: a Vulkan 1.1 query (checked by the caller) on a valid physical device with valid
    // chains.
    unsafe {
        instance.get_physical_device_image_format_properties2(physical, &info, &mut properties)
    }
    .context("The GPU cannot share RGBA8 images")?;
    let max = properties.image_format_properties.max_extent;
    if width > max.width || height > max.height {
        bail!("The GPU cannot share images of {width}x{height}");
    }
    if !external
        .external_memory_properties
        .external_memory_features
        .contains(vk::ExternalMemoryFeatureFlags::EXPORTABLE)
    {
        bail!("The GPU cannot export the memory of RGBA8 images");
    }
    Ok(usage)
}

/// Fails unless the device's semaphores can be exported as opaque file descriptors.
fn check_semaphore_export(instance: &ash::Instance, physical: vk::PhysicalDevice) -> Result<()> {
    let info = vk::PhysicalDeviceExternalSemaphoreInfo::default().handle_type(SEMAPHORE_HANDLE);
    let mut properties = vk::ExternalSemaphoreProperties::default();
    // SAFETY: a Vulkan 1.1 query (checked by the caller) on a valid physical device.
    unsafe {
        instance.get_physical_device_external_semaphore_properties(
            physical,
            &info,
            &mut properties,
        );
    }
    if !properties
        .external_semaphore_features
        .contains(vk::ExternalSemaphoreFeatureFlags::EXPORTABLE)
    {
        bail!("The GPU cannot export semaphores");
    }
    Ok(())
}

/// A command buffer with nothing but `barrier` in it, recorded on the raw device because wgpu has
/// no queue family transfers.
fn barrier(
    device: &wgpu::Device,
    raw_device: &ash::Device,
    src_stage: vk::PipelineStageFlags,
    dst_stage: vk::PipelineStageFlags,
    barrier: vk::ImageMemoryBarrier<'_>,
) -> Result<wgpu::CommandBuffer> {
    let mut encoder = device.create_command_encoder(&Default::default());
    // SAFETY: the encoder is used for nothing else and is left open for wgpu to finish; the
    // barrier names an image of this device, whose owner and layout it keeps consistent with
    // what wgpu's own barriers assume (see the module documentation).
    let recorded = unsafe {
        encoder.as_hal_mut::<hal::api::Vulkan, _, _>(|encoder| {
            encoder.map(|encoder| {
                raw_device.cmd_pipeline_barrier(
                    encoder.raw_handle(),
                    src_stage,
                    dst_stage,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
            })
        })
    };
    recorded.context("The GPU encoder does not run on Vulkan")?;
    Ok(encoder.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame drawn into the next image with a clear and finished by the GPU, or `None` if the
    /// pool is full.
    fn draw(
        targets: &mut SharedTargets,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<SharedFrame> {
        let next = targets.next(device).unwrap()?;
        let frame = targets
            .render(device, queue, next, |encoder, view| {
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            })
            .unwrap()
            .expect("wgpu refused the submission");
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        targets.keep(&frame);
        Some(frame)
    }

    /// Frames go into images that no held or recent frame is in, export their memory and
    /// semaphore, and are skipped once every image is held.
    #[test]
    fn frames_avoid_held_images_and_export() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let (device, queue) = match request_device(&adapter) {
            Ok(device) => device,
            Err(e) => {
                eprintln!("SKIPPED: {e:#}");
                return;
            }
        };
        let mut targets = SharedTargets::new(&device, 64, 32).unwrap();

        let mut recent: VecDeque<u64> = VecDeque::new();
        for _ in 0..2 * MAX_IMAGES {
            let frame = draw(&mut targets, &device, &queue).expect("an image is free");
            assert!(!recent.contains(&frame.image_id()));
            assert_eq!(frame.image_set(), targets.set);
            assert!(frame.memory_size() >= 64 * 32 * 4);
            frame.export_memory().unwrap();
            frame.export_semaphore().unwrap();
            recent.push_back(frame.image_id());
            if recent.len() > KEEP_RECENT {
                recent.pop_front();
            }
        }

        let held: Vec<SharedFrame> = std::iter::from_fn(|| draw(&mut targets, &device, &queue))
            .take(MAX_IMAGES + 1)
            .collect();
        assert!(held.len() <= MAX_IMAGES);
        assert!(draw(&mut targets, &device, &queue).is_none());
        drop(held);
    }
}
