//! Direct Vulkan rendering. Vulkan handles are owned here; callers never manage their lifetime.
use anyhow::{Context, Result, bail};
use ash::{Device, Entry, Instance, vk};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{ffi::CStr, io::Cursor, path::Path, time::Instant};
use winit::window::Window;

pub use glam;
const FRAMES: usize = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

#[derive(Default)]
struct Buffer {
    handle: vk::Buffer,
    memory: vk::DeviceMemory,
    capacity: usize,
}
struct Frame {
    available: vk::Semaphore,
    fence: vk::Fence,
    command: vk::CommandBuffer,
    submitted: bool,
    vertices: Buffer,
    revision: u64,
    vertex_count: u32,
    entities: Buffer,
    entity_count: u32,
}
#[derive(Default, Clone, Copy, Debug)]
pub struct FrameTiming {
    pub gpu_us: Option<f64>,
    pub submitted: bool,
    /// CPU wall time spent waiting for this frame slot's GPU fence. This is not
    /// GPU execution time or input-to-photon latency.
    pub fence_wait_us: f64,
}

pub struct Renderer {
    _entry: Entry,
    instance: Instance,
    surface_api: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical: vk::PhysicalDevice,
    device: Device,
    queue: vk::Queue,
    swap_api: ash::khr::swapchain::Device,
    swap: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    finished: Vec<vk::Semaphore>,
    depth_images: Vec<vk::Image>,
    depth_memory: Vec<vk::DeviceMemory>,
    depth_views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
    pass: vk::RenderPass,
    format: vk::Format,
    depth_format: vk::Format,
    pub extent: vk::Extent2D,
    pub gpu_name: String,
    pub api_version: String,
    pool: vk::CommandPool,
    frames: Vec<Frame>,
    slot: usize,
    vsync: bool,
    background_pipeline: vk::Pipeline,
    background_layout: vk::PipelineLayout,
    terrain_pipeline: vk::Pipeline,
    terrain_layout: vk::PipelineLayout,
    egui: Option<egui_ash_renderer::Renderer>,
    queries: vk::QueryPool,
    timestamp_period: f64,
    timestamp_bits: u32,
    pub last_timing: FrameTiming,
    can_capture: bool,
    frame_prepared: bool,
}

impl Renderer {
    pub fn new(window: &Window, vsync: bool) -> Result<Self> {
        // SAFETY: the loader remains owned until every instance/device handle is destroyed.
        unsafe {
            let entry =
                Entry::load().context("Vulkan loader is unavailable; install a GPU driver")?;
            let app = vk::ApplicationInfo::default()
                .application_name(c"Void.rs")
                .engine_name(c"Event Horizon")
                .api_version(vk::make_api_version(0, 1, 4, 0));
            let extensions =
                ash_window::enumerate_required_extensions(window.display_handle()?.as_raw())?;
            let instance = entry
                .create_instance(
                    &vk::InstanceCreateInfo::default()
                        .application_info(&app)
                        .enabled_extension_names(extensions),
                    None,
                )
                .context("Vulkan 1.4 instance creation")?;
            let surface_api = ash::khr::surface::Instance::new(&entry, &instance);
            let surface = match ash_window::create_surface(
                &entry,
                &instance,
                window.display_handle()?.as_raw(),
                window.window_handle()?.as_raw(),
                None,
            ) {
                Ok(surface) => surface,
                Err(error) => {
                    instance.destroy_instance(None);
                    return Err(error.into());
                }
            };
            let devices = match instance.enumerate_physical_devices() {
                Ok(devices) => devices,
                Err(error) => {
                    surface_api.destroy_surface(surface, None);
                    instance.destroy_instance(None);
                    return Err(error.into());
                }
            };
            let selection = devices
                .into_iter()
                .filter_map(|physical| {
                    let properties = instance.get_physical_device_properties(physical);
                    if properties.api_version < vk::make_api_version(0, 1, 4, 0) {
                        return None;
                    }
                    instance
                        .get_physical_device_queue_family_properties(physical)
                        .iter()
                        .enumerate()
                        .find(|(i, q)| {
                            q.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                                && surface_api
                                    .get_physical_device_surface_support(
                                        physical, *i as u32, surface,
                                    )
                                    .unwrap_or(false)
                        })
                        .map(|(i, q)| (physical, properties, i as u32, q.timestamp_valid_bits))
                })
                .max_by_key(|(_, p, _, _)| {
                    if p.device_type == vk::PhysicalDeviceType::DISCRETE_GPU {
                        2
                    } else {
                        1
                    }
                });
            let Some((physical, properties, family, timestamp_bits)) = selection else {
                surface_api.destroy_surface(surface, None);
                instance.destroy_instance(None);
                bail!("No Vulkan 1.4 GPU with graphics/presentation support was found");
            };
            let Some(depth_format) = [vk::Format::D32_SFLOAT, vk::Format::D16_UNORM]
                .into_iter()
                .find(|format| {
                    instance
                        .get_physical_device_format_properties(physical, *format)
                        .optimal_tiling_features
                        .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT)
                })
            else {
                surface_api.destroy_surface(surface, None);
                instance.destroy_instance(None);
                bail!("GPU has no supported depth attachment format");
            };
            let priorities = [1.0];
            let queues = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&priorities)];
            let extension_names = [ash::khr::swapchain::NAME.as_ptr()];
            let device = match instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&extension_names),
                None,
            ) {
                Ok(device) => device,
                Err(err) => {
                    surface_api.destroy_surface(surface, None);
                    instance.destroy_instance(None);
                    return Err(err.into());
                }
            };
            let queue = device.get_device_queue(family, 0);
            let swap_api = ash::khr::swapchain::Device::new(&instance, &device);
            let mut out = Self {
                _entry: entry,
                instance,
                surface_api,
                surface,
                physical,
                device,
                queue,
                swap_api,
                swap: vk::SwapchainKHR::null(),
                images: vec![],
                views: vec![],
                finished: vec![],
                depth_images: vec![],
                depth_memory: vec![],
                depth_views: vec![],
                framebuffers: vec![],
                pass: vk::RenderPass::null(),
                format: vk::Format::UNDEFINED,
                depth_format,
                extent: vk::Extent2D::default(),
                gpu_name: CStr::from_ptr(properties.device_name.as_ptr())
                    .to_string_lossy()
                    .into_owned(),
                api_version: format!(
                    "{}.{}.{}",
                    vk::api_version_major(properties.api_version),
                    vk::api_version_minor(properties.api_version),
                    vk::api_version_patch(properties.api_version)
                ),
                pool: vk::CommandPool::null(),
                frames: vec![],
                slot: 0,
                vsync,
                background_pipeline: vk::Pipeline::null(),
                background_layout: vk::PipelineLayout::null(),
                terrain_pipeline: vk::Pipeline::null(),
                terrain_layout: vk::PipelineLayout::null(),
                egui: None,
                queries: vk::QueryPool::null(),
                timestamp_period: properties.limits.timestamp_period as f64,
                timestamp_bits,
                last_timing: FrameTiming::default(),
                can_capture: false,
                frame_prepared: false,
            };
            out.pool = out.device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;
            let commands = out.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(out.pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(FRAMES as u32),
            )?;
            for command in commands {
                let available = out
                    .device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
                let fence = match out.device.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                ) {
                    Ok(fence) => fence,
                    Err(error) => {
                        out.device.destroy_semaphore(available, None);
                        return Err(error.into());
                    }
                };
                out.frames.push(Frame {
                    available,
                    fence,
                    command,
                    submitted: false,
                    vertices: Buffer::default(),
                    revision: u64::MAX,
                    vertex_count: 0,
                    entities: Buffer::default(),
                    entity_count: 0,
                });
            }
            if timestamp_bits > 0 {
                out.queries = out.device.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count((FRAMES * 2) as u32),
                    None,
                )?;
            }
            out.create_swapchain(window)?;
            out.create_pipelines()?;
            out.egui = Some(egui_ash_renderer::Renderer::with_default_allocator(
                &out.instance,
                out.physical,
                out.device.clone(),
                out.pass,
                egui_ash_renderer::Options {
                    in_flight_frames: FRAMES,
                    srgb_framebuffer: is_srgb(out.format),
                    ..Default::default()
                },
            )?);
            Ok(out)
        }
    }

    fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32> {
        let props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical)
        };
        (0..props.memory_type_count)
            .find(|i| {
                bits & (1 << i) != 0
                    && props.memory_types[*i as usize]
                        .property_flags
                        .contains(flags)
            })
            .context("GPU lacks required memory type")
    }
    fn buffer(&self, bytes: usize, usage: vk::BufferUsageFlags) -> Result<Buffer> {
        unsafe {
            let handle = self.device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(bytes as u64)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?;
            let req = self.device.get_buffer_memory_requirements(handle);
            let memory_type = match self.memory_type(
                req.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            ) {
                Ok(index) => index,
                Err(error) => {
                    self.device.destroy_buffer(handle, None);
                    return Err(error);
                }
            };
            let memory = match self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(memory_type),
                None,
            ) {
                Ok(memory) => memory,
                Err(error) => {
                    self.device.destroy_buffer(handle, None);
                    return Err(error.into());
                }
            };
            if let Err(error) = self.device.bind_buffer_memory(handle, memory, 0) {
                self.device.destroy_buffer(handle, None);
                self.device.free_memory(memory, None);
                return Err(error.into());
            }
            Ok(Buffer {
                handle,
                memory,
                capacity: bytes,
            })
        }
    }
    fn destroy_buffer(&self, b: &Buffer) {
        unsafe {
            self.device.destroy_buffer(b.handle, None);
            self.device.free_memory(b.memory, None);
        }
    }
    fn create_swapchain(&mut self, window: &Window) -> Result<()> {
        unsafe {
            let caps = self
                .surface_api
                .get_physical_device_surface_capabilities(self.physical, self.surface)?;
            let formats = self
                .surface_api
                .get_physical_device_surface_formats(self.physical, self.surface)?;
            let chosen = [
                vk::Format::B8G8R8A8_SRGB,
                vk::Format::R8G8B8A8_SRGB,
                vk::Format::B8G8R8A8_UNORM,
                vk::Format::R8G8B8A8_UNORM,
            ]
            .into_iter()
            .find_map(|format| formats.iter().find(|f| f.format == format).copied())
            .or_else(|| formats.first().copied())
            .context("No surface format")?;
            let chosen = if chosen.format == vk::Format::UNDEFINED {
                vk::SurfaceFormatKHR {
                    format: vk::Format::B8G8R8A8_SRGB,
                    color_space: chosen.color_space,
                }
            } else {
                chosen
            };
            if self.format != vk::Format::UNDEFINED && self.format != chosen.format {
                bail!("Surface format changed; restart the client");
            }
            self.format = chosen.format;
            let modes = self
                .surface_api
                .get_physical_device_surface_present_modes(self.physical, self.surface)?;
            let mode = if self.vsync {
                vk::PresentModeKHR::FIFO
            } else if modes.contains(&vk::PresentModeKHR::MAILBOX) {
                vk::PresentModeKHR::MAILBOX
            } else if modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
                vk::PresentModeKHR::IMMEDIATE
            } else {
                vk::PresentModeKHR::FIFO
            };
            let size = window.inner_size();
            self.extent = if caps.current_extent.width != u32::MAX {
                caps.current_extent
            } else {
                vk::Extent2D {
                    width: size
                        .width
                        .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                    height: size
                        .height
                        .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
                }
            };
            let mut count = caps.min_image_count.max(3);
            if caps.max_image_count > 0 {
                count = count.min(caps.max_image_count);
            }
            self.can_capture = caps
                .supported_usage_flags
                .contains(vk::ImageUsageFlags::TRANSFER_SRC)
                && is_rgba8(self.format);
            let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT
                | if self.can_capture {
                    vk::ImageUsageFlags::TRANSFER_SRC
                } else {
                    vk::ImageUsageFlags::empty()
                };
            let composite = [
                vk::CompositeAlphaFlagsKHR::OPAQUE,
                vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::INHERIT,
            ]
            .into_iter()
            .find(|v| caps.supported_composite_alpha.contains(*v))
            .context("No compositor alpha mode")?;
            self.swap = self.swap_api.create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(self.surface)
                    .min_image_count(count)
                    .image_format(chosen.format)
                    .image_color_space(chosen.color_space)
                    .image_extent(self.extent)
                    .image_array_layers(1)
                    .image_usage(usage)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(composite)
                    .present_mode(mode)
                    .clipped(true),
                None,
            )?;
            self.images = self.swap_api.get_swapchain_images(self.swap)?;
            if self.pass == vk::RenderPass::null() {
                let attachments = [
                    vk::AttachmentDescription::default()
                        .format(self.format)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .load_op(vk::AttachmentLoadOp::CLEAR)
                        .store_op(vk::AttachmentStoreOp::STORE)
                        .initial_layout(vk::ImageLayout::UNDEFINED)
                        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR),
                    vk::AttachmentDescription::default()
                        .format(self.depth_format)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .load_op(vk::AttachmentLoadOp::CLEAR)
                        .store_op(vk::AttachmentStoreOp::DONT_CARE)
                        .initial_layout(vk::ImageLayout::UNDEFINED)
                        .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
                ];
                let colors = [vk::AttachmentReference::default()
                    .attachment(0)
                    .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
                let depth = vk::AttachmentReference::default()
                    .attachment(1)
                    .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
                let subpasses = [vk::SubpassDescription::default()
                    .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                    .color_attachments(&colors)
                    .depth_stencil_attachment(&depth)];
                let deps = [vk::SubpassDependency::default()
                    .src_subpass(vk::SUBPASS_EXTERNAL)
                    .dst_subpass(0)
                    .src_stage_mask(
                        vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                            | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                    )
                    .dst_stage_mask(
                        vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                            | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                    )
                    .src_access_mask(
                        vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                    )
                    .dst_access_mask(
                        vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE
                            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ,
                    )];
                self.pass = self.device.create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&attachments)
                        .subpasses(&subpasses)
                        .dependencies(&deps),
                    None,
                )?;
            }
            for image in &self.images {
                self.views.push(
                    self.device.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(*image)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(self.format)
                            .subresource_range(color_range()),
                        None,
                    )?,
                );
                let depth = self.device.create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(self.depth_format)
                        .extent(vk::Extent3D {
                            width: self.extent.width,
                            height: self.extent.height,
                            depth: 1,
                        })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
                        .initial_layout(vk::ImageLayout::UNDEFINED),
                    None,
                )?;
                self.depth_images.push(depth);
                let req = self.device.get_image_memory_requirements(depth);
                let memory = self.device.allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(req.size)
                        .memory_type_index(self.memory_type(
                            req.memory_type_bits,
                            vk::MemoryPropertyFlags::DEVICE_LOCAL,
                        )?),
                    None,
                )?;
                self.depth_memory.push(memory);
                self.device.bind_image_memory(depth, memory, 0)?;
                self.depth_views.push(
                    self.device.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(depth)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(self.depth_format)
                            .subresource_range(
                                vk::ImageSubresourceRange::default()
                                    .aspect_mask(vk::ImageAspectFlags::DEPTH)
                                    .level_count(1)
                                    .layer_count(1),
                            ),
                        None,
                    )?,
                );
                let attachments = [
                    *self.views.last().unwrap(),
                    *self.depth_views.last().unwrap(),
                ];
                self.framebuffers.push(
                    self.device.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(self.pass)
                            .attachments(&attachments)
                            .width(self.extent.width)
                            .height(self.extent.height)
                            .layers(1),
                        None,
                    )?,
                );
                self.finished.push(
                    self.device
                        .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?,
                );
            }
        }
        Ok(())
    }
    fn create_pipelines(&mut self) -> Result<()> {
        self.background_layout = unsafe {
            self.device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&[
                    vk::PushConstantRange::default()
                        .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                        .size(16),
                ]),
                None,
            )?
        };
        self.terrain_layout = unsafe {
            self.device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&[
                    vk::PushConstantRange::default()
                        .stage_flags(vk::ShaderStageFlags::VERTEX)
                        .size(64),
                ]),
                None,
            )?
        };
        self.background_pipeline = self.pipeline(
            include_bytes!(concat!(env!("OUT_DIR"), "/background.vert.spv")),
            include_bytes!(concat!(env!("OUT_DIR"), "/background.frag.spv")),
            self.background_layout,
            false,
        )?;
        self.terrain_pipeline = self.pipeline(
            include_bytes!(concat!(env!("OUT_DIR"), "/terrain.vert.spv")),
            include_bytes!(concat!(env!("OUT_DIR"), "/terrain.frag.spv")),
            self.terrain_layout,
            true,
        )?;
        Ok(())
    }
    fn pipeline(
        &self,
        vs: &[u8],
        fs: &[u8],
        layout: vk::PipelineLayout,
        terrain: bool,
    ) -> Result<vk::Pipeline> {
        unsafe {
            let vertex_code = ash::util::read_spv(&mut Cursor::new(vs))?;
            let fragment_code = ash::util::read_spv(&mut Cursor::new(fs))?;
            let vertex = self.device.create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&vertex_code),
                None,
            )?;
            let fragment = match self.device.create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&fragment_code),
                None,
            ) {
                Ok(fragment) => fragment,
                Err(error) => {
                    self.device.destroy_shader_module(vertex, None);
                    return Err(error.into());
                }
            };
            let stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(vertex)
                    .name(c"main"),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(fragment)
                    .name(c"main"),
            ];
            let binding = [vk::VertexInputBindingDescription {
                binding: 0,
                stride: 24,
                input_rate: vk::VertexInputRate::VERTEX,
            }];
            let attrs = [
                vk::VertexInputAttributeDescription {
                    location: 0,
                    binding: 0,
                    format: vk::Format::R32G32B32_SFLOAT,
                    offset: 0,
                },
                vk::VertexInputAttributeDescription {
                    location: 1,
                    binding: 0,
                    format: vk::Format::R32G32B32_SFLOAT,
                    offset: 12,
                },
            ];
            let input = if terrain {
                vk::PipelineVertexInputStateCreateInfo::default()
                    .vertex_binding_descriptions(&binding)
                    .vertex_attribute_descriptions(&attrs)
            } else {
                vk::PipelineVertexInputStateCreateInfo::default()
            };
            let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
            let viewport = vk::PipelineViewportStateCreateInfo::default()
                .viewport_count(1)
                .scissor_count(1);
            let raster = vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(vk::CullModeFlags::NONE)
                .line_width(1.0);
            let multi = vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(vk::SampleCountFlags::TYPE_1);
            let depth = vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(terrain)
                .depth_write_enable(terrain)
                .depth_compare_op(vk::CompareOp::LESS);
            let color = [vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA)];
            let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&color);
            let dynamic = vk::PipelineDynamicStateCreateInfo::default()
                .dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);
            let result = self.device.create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&input)
                    .input_assembly_state(&assembly)
                    .viewport_state(&viewport)
                    .rasterization_state(&raster)
                    .multisample_state(&multi)
                    .depth_stencil_state(&depth)
                    .color_blend_state(&blend)
                    .dynamic_state(&dynamic)
                    .layout(layout)
                    .render_pass(self.pass)],
                None,
            );
            self.device.destroy_shader_module(vertex, None);
            self.device.destroy_shader_module(fragment, None);
            match result {
                Ok(pipelines) => Ok(pipelines[0]),
                Err((partial, error)) => {
                    for pipeline in partial {
                        self.device.destroy_pipeline(pipeline, None);
                    }
                    Err(error.into())
                }
            }
        }
    }

    /// Nonblocking readiness check for event-loop coordination. While false, the
    /// caller can continue receiving input and network events before redrawing.
    pub fn is_frame_ready(&self) -> Result<bool> {
        if self.frame_prepared {
            return Ok(true);
        }
        Ok(unsafe { self.device.get_fence_status(self.frames[self.slot].fence)? })
    }

    /// Wait before sampling UI/camera input. The fence protects this frame's buffers.
    pub fn prepare_frame(&mut self) -> Result<FrameTiming> {
        if self.frame_prepared {
            return Ok(self.last_timing);
        }
        let wait_started = Instant::now();
        unsafe {
            self.device
                .wait_for_fences(&[self.frames[self.slot].fence], true, u64::MAX)?;
        }
        let mut timing = FrameTiming {
            fence_wait_us: wait_started.elapsed().as_secs_f64() * 1_000_000.0,
            ..Default::default()
        };
        if self.frames[self.slot].submitted && self.queries != vk::QueryPool::null() {
            let mut ticks = [0_u64; 2];
            if unsafe {
                self.device.get_query_pool_results(
                    self.queries,
                    (self.slot * 2) as u32,
                    &mut ticks,
                    vk::QueryResultFlags::TYPE_64,
                )
            }
            .is_ok()
            {
                let mask = if self.timestamp_bits == 64 {
                    u64::MAX
                } else {
                    (1_u64 << self.timestamp_bits) - 1
                };
                timing.gpu_us = Some(
                    (ticks[1].wrapping_sub(ticks[0]) & mask) as f64 * self.timestamp_period
                        / 1000.0,
                );
            }
        }
        self.frames[self.slot].entity_count = 0;
        self.frame_prepared = true;
        self.last_timing = timing;
        Ok(timing)
    }
    pub fn resize(&mut self, window: &Window, vsync: bool) -> Result<()> {
        if window.inner_size().width == 0 || window.inner_size().height == 0 {
            return Ok(());
        }
        unsafe {
            self.device.device_wait_idle()?;
        }
        self.frame_prepared = false;
        self.destroy_swapchain();
        self.vsync = vsync;
        self.create_swapchain(window)
    }
    /// Uploads only when this slot's mesh revision changes; old buffers live until their fence completes.
    pub fn update_mesh(&mut self, revision: u64, vertices: &[Vertex]) -> Result<()> {
        if !self.frame_prepared {
            self.prepare_frame()?;
        }
        if self.frames[self.slot].revision == revision {
            return Ok(());
        }
        if vertices.len() > 4_000_000 {
            bail!("Terrain mesh exceeds the 96 MiB upload limit");
        }
        let bytes: &[u8] = bytemuck::cast_slice(vertices);
        if bytes.len() > self.frames[self.slot].vertices.capacity {
            let new = self.buffer(
                bytes.len().max(1024).next_power_of_two(),
                vk::BufferUsageFlags::VERTEX_BUFFER,
            )?;
            let old = std::mem::replace(&mut self.frames[self.slot].vertices, new);
            self.destroy_buffer(&old);
        }
        if !bytes.is_empty() {
            unsafe {
                let ptr = self.device.map_memory(
                    self.frames[self.slot].vertices.memory,
                    0,
                    bytes.len() as u64,
                    vk::MemoryMapFlags::empty(),
                )?;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), bytes.len());
                self.device
                    .unmap_memory(self.frames[self.slot].vertices.memory);
            }
        }
        self.frames[self.slot].revision = revision;
        self.frames[self.slot].vertex_count = vertices.len() as u32;
        Ok(())
    }

    /// Upload dynamic geometry into a separate fence-protected per-frame buffer.
    /// Call after prepare_frame on every frame that needs entities; omitted calls
    /// leave dynamic geometry empty rather than reusing a stale world's entities.
    pub fn update_entities(&mut self, vertices: &[Vertex]) -> Result<()> {
        if !self.frame_prepared {
            self.prepare_frame()?;
        }
        if vertices.len() > 1_000_000 {
            bail!("Entity mesh exceeds the 24 MiB upload limit");
        }
        let bytes: &[u8] = bytemuck::cast_slice(vertices);
        if bytes.len() > self.frames[self.slot].entities.capacity {
            let new = self.buffer(
                bytes.len().max(1024).next_power_of_two(),
                vk::BufferUsageFlags::VERTEX_BUFFER,
            )?;
            let old = std::mem::replace(&mut self.frames[self.slot].entities, new);
            self.destroy_buffer(&old);
        }
        if !bytes.is_empty() {
            unsafe {
                let memory = self.frames[self.slot].entities.memory;
                let ptr = self.device.map_memory(
                    memory,
                    0,
                    bytes.len() as u64,
                    vk::MemoryMapFlags::empty(),
                )?;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), bytes.len());
                self.device.unmap_memory(memory);
            }
        }
        self.frames[self.slot].entity_count = vertices.len() as u32;
        Ok(())
    }

    pub fn render(
        &mut self,
        window: &Window,
        ctx: &egui::Context,
        output: egui::FullOutput,
        camera: Option<glam::Mat4>,
        capture: Option<&Path>,
    ) -> Result<bool> {
        if window.inner_size().width == 0 || window.inner_size().height == 0 {
            return Ok(false);
        }
        if !self.frame_prepared {
            self.prepare_frame()?;
        }
        if capture.is_some() && !self.can_capture {
            bail!(
                "Screenshot capture is unavailable for this surface usage/format ({:?})",
                self.format
            );
        }
        let acquired = unsafe {
            self.swap_api.acquire_next_image(
                self.swap,
                u64::MAX,
                self.frames[self.slot].available,
                vk::Fence::null(),
            )
        };
        let (index, suboptimal) = match acquired {
            Ok(v) => v,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.resize(window, self.vsync)?;
                return Ok(false);
            }
            Err(e) => return Err(e.into()),
        };
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        // Managed texture mutation is uncommon (fonts/asset changes). Synchronize all users explicitly.
        if !output.textures_delta.set.is_empty() || !output.textures_delta.free.is_empty() {
            unsafe {
                self.device.device_wait_idle()?;
            }
            let ui = self.egui.as_mut().unwrap();
            ui.set_textures(self.queue, self.pool, &output.textures_delta.set)?;
        }
        let cmd = self.frames[self.slot].command;
        let readback = if capture.is_some() {
            let size = (self.extent.width as usize)
                .checked_mul(self.extent.height as usize)
                .and_then(|size| size.checked_mul(4))
                .context("Screenshot extent overflows buffer size")?;
            Some(self.buffer(size, vk::BufferUsageFlags::TRANSFER_DST)?)
        } else {
            None
        };
        let result = (|| -> Result<()> {
            unsafe {
                self.device
                    .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
                self.device
                    .begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default())?;
                if self.queries != vk::QueryPool::null() {
                    self.device
                        .cmd_reset_query_pool(cmd, self.queries, (self.slot * 2) as u32, 2);
                    self.device.cmd_write_timestamp(
                        cmd,
                        vk::PipelineStageFlags::TOP_OF_PIPE,
                        self.queries,
                        (self.slot * 2) as u32,
                    );
                }
                let clears = [
                    vk::ClearValue {
                        color: vk::ClearColorValue {
                            float32: if camera.is_some() {
                                [0.22, 0.36, 0.50, 1.0]
                            } else {
                                [0.005, 0.008, 0.014, 1.0]
                            },
                        },
                    },
                    vk::ClearValue {
                        depth_stencil: vk::ClearDepthStencilValue {
                            depth: 1.0,
                            stencil: 0,
                        },
                    },
                ];
                self.device.cmd_begin_render_pass(
                    cmd,
                    &vk::RenderPassBeginInfo::default()
                        .render_pass(self.pass)
                        .framebuffer(self.framebuffers[index as usize])
                        .render_area(vk::Rect2D {
                            offset: vk::Offset2D::default(),
                            extent: self.extent,
                        })
                        .clear_values(&clears),
                    vk::SubpassContents::INLINE,
                );
                self.device.cmd_set_viewport(
                    cmd,
                    0,
                    &[vk::Viewport {
                        x: 0.0,
                        y: 0.0,
                        width: self.extent.width as f32,
                        height: self.extent.height as f32,
                        min_depth: 0.0,
                        max_depth: 1.0,
                    }],
                );
                self.device.cmd_set_scissor(
                    cmd,
                    0,
                    &[vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent: self.extent,
                    }],
                );
                if let Some(matrix) = camera {
                    let frame = &self.frames[self.slot];
                    if frame.vertex_count > 0 || frame.entity_count > 0 {
                        self.device.cmd_bind_pipeline(
                            cmd,
                            vk::PipelineBindPoint::GRAPHICS,
                            self.terrain_pipeline,
                        );
                        self.device.cmd_push_constants(
                            cmd,
                            self.terrain_layout,
                            vk::ShaderStageFlags::VERTEX,
                            0,
                            bytemuck::cast_slice(&matrix.to_cols_array()),
                        );
                        if frame.vertex_count > 0 {
                            self.device.cmd_bind_vertex_buffers(
                                cmd,
                                0,
                                &[frame.vertices.handle],
                                &[0],
                            );
                            self.device.cmd_draw(cmd, frame.vertex_count, 1, 0, 0);
                        }
                        if frame.entity_count > 0 {
                            self.device.cmd_bind_vertex_buffers(
                                cmd,
                                0,
                                &[frame.entities.handle],
                                &[0],
                            );
                            self.device.cmd_draw(cmd, frame.entity_count, 1, 0, 0);
                        }
                    }
                } else {
                    self.device.cmd_bind_pipeline(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.background_pipeline,
                    );
                    self.device.cmd_push_constants(
                        cmd,
                        self.background_layout,
                        vk::ShaderStageFlags::FRAGMENT,
                        0,
                        bytemuck::cast_slice(&[
                            self.extent.width as f32 / self.extent.height as f32,
                            0.0,
                            0.0,
                            0.0,
                        ]),
                    );
                    self.device.cmd_draw(cmd, 3, 1, 0, 0);
                }
                self.egui.as_mut().unwrap().cmd_draw(
                    cmd,
                    self.extent,
                    output.pixels_per_point,
                    &primitives,
                )?;
                self.device.cmd_end_render_pass(cmd);
                if let Some(ref readback) = readback {
                    let barrier = vk::ImageMemoryBarrier::default()
                        .image(self.images[index as usize])
                        .subresource_range(color_range())
                        .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED);
                    self.device.cmd_pipeline_barrier(
                        cmd,
                        vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier],
                    );
                    self.device.cmd_copy_image_to_buffer(
                        cmd,
                        self.images[index as usize],
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        readback.handle,
                        &[vk::BufferImageCopy::default()
                            .image_subresource(
                                vk::ImageSubresourceLayers::default()
                                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                                    .layer_count(1),
                            )
                            .image_extent(vk::Extent3D {
                                width: self.extent.width,
                                height: self.extent.height,
                                depth: 1,
                            })],
                    );
                    // A fence guarantees execution completion, but host-coherent
                    // memory still requires device writes to become host-visible.
                    let host_read = vk::BufferMemoryBarrier::default()
                        .buffer(readback.handle)
                        .offset(0)
                        .size(readback.capacity as u64)
                        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                        .dst_access_mask(vk::AccessFlags::HOST_READ)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED);
                    self.device.cmd_pipeline_barrier(
                        cmd,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::HOST,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[host_read],
                        &[],
                    );
                    let restore = barrier
                        .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .dst_access_mask(vk::AccessFlags::empty());
                    self.device.cmd_pipeline_barrier(
                        cmd,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[restore],
                    );
                }
                if self.queries != vk::QueryPool::null() {
                    self.device.cmd_write_timestamp(
                        cmd,
                        vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                        self.queries,
                        (self.slot * 2 + 1) as u32,
                    );
                }
                self.device.end_command_buffer(cmd)?;
                let wait = [self.frames[self.slot].available];
                let stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS];
                let commands = [cmd];
                let signal = [self.finished[index as usize]];
                self.device.reset_fences(&[self.frames[self.slot].fence])?;
                self.device.queue_submit(
                    self.queue,
                    &[vk::SubmitInfo::default()
                        .wait_semaphores(&wait)
                        .wait_dst_stage_mask(&stages)
                        .command_buffers(&commands)
                        .signal_semaphores(&signal)],
                    self.frames[self.slot].fence,
                )?;
                self.frames[self.slot].submitted = true;
                self.frame_prepared = false;
                if let (Some(path), Some(readback)) = (capture, readback.as_ref()) {
                    self.device
                        .wait_for_fences(&[self.frames[self.slot].fence], true, u64::MAX)?;
                    let ptr = self.device.map_memory(
                        readback.memory,
                        0,
                        readback.capacity as u64,
                        vk::MemoryMapFlags::empty(),
                    )?;
                    let mut pixels =
                        std::slice::from_raw_parts(ptr.cast::<u8>(), readback.capacity).to_vec();
                    self.device.unmap_memory(readback.memory);
                    if self.format == vk::Format::B8G8R8A8_SRGB
                        || self.format == vk::Format::B8G8R8A8_UNORM
                    {
                        for p in pixels.as_chunks_mut::<4>().0 {
                            p.swap(0, 2);
                        }
                    }
                    image::save_buffer(
                        path,
                        &pixels,
                        self.extent.width,
                        self.extent.height,
                        image::ColorType::Rgba8,
                    )?;
                }
                let swaps = [self.swap];
                let indices = [index];
                let present = self.swap_api.queue_present(
                    self.queue,
                    &vk::PresentInfoKHR::default()
                        .wait_semaphores(&signal)
                        .swapchains(&swaps)
                        .image_indices(&indices),
                );
                if !output.textures_delta.free.is_empty() {
                    self.device.device_wait_idle()?;
                    self.egui
                        .as_mut()
                        .unwrap()
                        .free_textures(&output.textures_delta.free)?;
                }
                self.slot = (self.slot + 1) % FRAMES;
                match present {
                    Ok(resize) if resize || suboptimal => self.resize(window, self.vsync)?,
                    Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.resize(window, self.vsync)?,
                    Err(e) => return Err(e.into()),
                    _ => {}
                }
            }
            Ok(())
        })();
        if let Some(readback) = readback {
            // Error paths may follow submission but precede normal fence wait.
            if result.is_err() {
                unsafe {
                    let _ = self.device.device_wait_idle();
                }
            }
            self.destroy_buffer(&readback);
        }
        result?;
        self.last_timing.submitted = true;
        Ok(true)
    }
    fn destroy_swapchain(&mut self) {
        unsafe {
            for h in self.framebuffers.drain(..) {
                self.device.destroy_framebuffer(h, None);
            }
            for h in self.views.drain(..) {
                self.device.destroy_image_view(h, None);
            }
            for h in self.depth_views.drain(..) {
                self.device.destroy_image_view(h, None);
            }
            for h in self.depth_images.drain(..) {
                self.device.destroy_image(h, None);
            }
            for h in self.depth_memory.drain(..) {
                self.device.free_memory(h, None);
            }
            for h in self.finished.drain(..) {
                self.device.destroy_semaphore(h, None);
            }
            self.swap_api.destroy_swapchain(self.swap, None);
            self.swap = vk::SwapchainKHR::null();
            self.images.clear();
        }
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.egui.take();
            self.device.destroy_pipeline(self.background_pipeline, None);
            self.device.destroy_pipeline(self.terrain_pipeline, None);
            self.device
                .destroy_pipeline_layout(self.background_layout, None);
            self.device
                .destroy_pipeline_layout(self.terrain_layout, None);
            self.destroy_swapchain();
            self.device.destroy_render_pass(self.pass, None);
            self.device.destroy_query_pool(self.queries, None);
            for frame in &self.frames {
                self.destroy_buffer(&frame.vertices);
                self.destroy_buffer(&frame.entities);
                self.device.destroy_semaphore(frame.available, None);
                self.device.destroy_fence(frame.fence, None);
            }
            self.device.destroy_command_pool(self.pool, None);
            self.device.destroy_device(None);
            self.surface_api.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}
fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1)
}
fn is_srgb(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB
    )
}
fn is_rgba8(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::B8G8R8A8_SRGB
            | vk::Format::R8G8B8A8_SRGB
            | vk::Format::B8G8R8A8_UNORM
            | vk::Format::R8G8B8A8_UNORM
    )
}
