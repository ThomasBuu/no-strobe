//! Vulkan compute acceleration for frame deflickering and row profile reduction.
//!
//! Provides GPU-accelerated:
//! - Full-frame deflickering & de-banding kernel (`deflicker.comp`)
//! - Parallel horizontal row luminance reduction (`row_reduction.comp`)
//!
//! Operates headlessly via `ash` dynamic loading (`libvulkan.so` / `vulkan-1.dll`).

use ash::vk;
use std::ffi::{CStr, CString};

pub struct VulkanBuffer {
    pub buffer: vk::Buffer,
    pub memory: vk::DeviceMemory,
    pub size: vk::DeviceSize,
    pub mapped_ptr: *mut u8,
    pub coherent: bool,
}

impl VulkanBuffer {
    pub fn new(
        device: &ash::Device,
        props: &vk::PhysicalDeviceMemoryProperties,
        size: usize,
        usage: vk::BufferUsageFlags,
    ) -> Result<Self, String> {
        let size = (size as vk::DeviceSize).max(64);
        let buf_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buffer = unsafe {
            device
                .create_buffer(&buf_info, None)
                .map_err(|e| format!("vkCreateBuffer failed: {e}"))?
        };

        let reqs = unsafe { device.get_buffer_memory_requirements(buffer) };

        let mut chosen_type = None;
        let mut is_coherent = false;
        let mut best_score = -1;

        for i in 0..props.memory_type_count {
            let matches_req = (reqs.memory_type_bits & (1 << i)) != 0;
            if !matches_req {
                continue;
            }
            let flags = props.memory_types[i as usize].property_flags;
            let is_host_visible = flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE);
            let is_cached = flags.contains(vk::MemoryPropertyFlags::HOST_CACHED);
            let is_coh = flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT);

            let score = if is_host_visible && is_cached && is_coh {
                30
            } else if is_host_visible && is_cached {
                20
            } else if is_host_visible && is_coh {
                10
            } else if is_host_visible {
                5
            } else {
                -1
            };
            if score > best_score {
                best_score = score;
                chosen_type = Some(i);
                is_coherent = is_coh;
            }
        }

        let type_index = chosen_type.ok_or_else(|| "No host-visible memory type found".to_string())?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(reqs.size)
            .memory_type_index(type_index);

        let memory = unsafe {
            device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("vkAllocateMemory failed: {e}"))?
        };

        unsafe {
            device
                .bind_buffer_memory(buffer, memory, 0)
                .map_err(|e| format!("vkBindBufferMemory failed: {e}"))?;
        }

        let mapped_ptr = unsafe {
            device
                .map_memory(memory, 0, reqs.size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("vkMapMemory failed: {e}"))? as *mut u8
        };

        Ok(Self {
            buffer,
            memory,
            size: reqs.size,
            mapped_ptr,
            coherent: is_coherent,
        })
    }

    pub fn write(&self, device: &ash::Device, data: &[u8]) {
        let copy_len = data.len().min(self.size as usize);
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.mapped_ptr, copy_len);
            if !self.coherent {
                let range = vk::MappedMemoryRange::default()
                    .memory(self.memory)
                    .offset(0)
                    .size(vk::WHOLE_SIZE);
                let _ = device.flush_mapped_memory_ranges(&[range]);
            }
        }
    }

    pub fn read(&self, device: &ash::Device, out: &mut [u8]) {
        let copy_len = out.len().min(self.size as usize);
        unsafe {
            if !self.coherent {
                let range = vk::MappedMemoryRange::default()
                    .memory(self.memory)
                    .offset(0)
                    .size(vk::WHOLE_SIZE);
                let _ = device.invalidate_mapped_memory_ranges(&[range]);
            }
            std::ptr::copy_nonoverlapping(self.mapped_ptr, out.as_mut_ptr(), copy_len);
        }
    }

    pub fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if !self.mapped_ptr.is_null() {
                device.unmap_memory(self.memory);
                self.mapped_ptr = std::ptr::null_mut();
            }
            if self.buffer != vk::Buffer::null() {
                device.destroy_buffer(self.buffer, None);
                self.buffer = vk::Buffer::null();
            }
            if self.memory != vk::DeviceMemory::null() {
                device.free_memory(self.memory, None);
                self.memory = vk::DeviceMemory::null();
            }
        }
    }
}

pub struct VulkanCompute {
    _entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
    _physical_device: vk::PhysicalDevice,
    queue: vk::Queue,
    _queue_family_index: u32,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,

    deflicker_shader: vk::ShaderModule,
    reduction_shader: vk::ShaderModule,

    deflicker_desc_layout: vk::DescriptorSetLayout,
    deflicker_pipe_layout: vk::PipelineLayout,
    deflicker_pipeline: vk::Pipeline,

    reduction_desc_layout: vk::DescriptorSetLayout,
    reduction_pipe_layout: vk::PipelineLayout,
    reduction_pipeline: vk::Pipeline,

    descriptor_pool: vk::DescriptorPool,
    deflicker_desc_set: vk::DescriptorSet,
    reduction_desc_set: vk::DescriptorSet,

    input_buffer: Option<VulkanBuffer>,
    output_buffer: Option<VulkanBuffer>,
    row_gains_buffer: Option<VulkanBuffer>,
    reduction_buffer: Option<VulkanBuffer>,

    memory_properties: vk::PhysicalDeviceMemoryProperties,
}

impl VulkanCompute {
    /// Attempts to initialize Vulkan compute on the system.
    pub fn new() -> Result<Self, String> {
        let entry = unsafe { ash::Entry::load().map_err(|e| format!("Cannot load Vulkan: {e}"))? };

        let app_name = CString::new("no-strobe").map_err(|e| e.to_string())?;
        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&app_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_1);

        let instance_info = vk::InstanceCreateInfo::default().application_info(&app_info);

        let instance = unsafe {
            entry
                .create_instance(&instance_info, None)
                .map_err(|e| format!("vkCreateInstance failed: {e}"))?
        };

        let physical_devices = unsafe {
            instance
                .enumerate_physical_devices()
                .map_err(|e| format!("vkEnumeratePhysicalDevices failed: {e}"))?
        };

        if physical_devices.is_empty() {
            unsafe { instance.destroy_instance(None) };
            return Err("No Vulkan physical devices found".into());
        }

        let mut chosen = None;
        let mut chosen_qf = 0u32;
        let mut best_score = -1;

        for &pd in &physical_devices {
            let props = unsafe { instance.get_physical_device_properties(pd) };
            let qfs = unsafe { instance.get_physical_device_queue_family_properties(pd) };

            for (idx, qf) in qfs.iter().enumerate() {
                if qf.queue_flags.contains(vk::QueueFlags::COMPUTE) {
                    let score = match props.device_type {
                        vk::PhysicalDeviceType::DISCRETE_GPU => 20,
                        vk::PhysicalDeviceType::INTEGRATED_GPU => 10,
                        _ => 1,
                    };
                    if score > best_score {
                        best_score = score;
                        chosen = Some(pd);
                        chosen_qf = idx as u32;
                    }
                    break;
                }
            }
        }

        let (physical_device, queue_family_index) = match chosen {
            Some(pd) => (pd, chosen_qf),
            None => {
                unsafe { instance.destroy_instance(None) };
                return Err("No compute queue found on any physical device".into());
            }
        };

        let priorities = [1.0f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priorities);
        let queue_infos = [queue_info];
        let device_info = vk::DeviceCreateInfo::default().queue_create_infos(&queue_infos);

        let device = unsafe {
            instance
                .create_device(physical_device, &device_info, None)
                .map_err(|e| {
                    instance.destroy_instance(None);
                    format!("vkCreateDevice failed: {e}")
                })?
        };

        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
        let memory_properties = unsafe { instance.get_physical_device_memory_properties(physical_device) };

        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = unsafe {
            device
                .create_command_pool(&pool_info, None)
                .map_err(|e| format!("vkCreateCommandPool failed: {e}"))?
        };

        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffers = unsafe {
            device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("vkAllocateCommandBuffers failed: {e}"))?
        };
        let command_buffer = command_buffers[0];

        // Compile or load shader modules
        let deflicker_spv = include_bytes!("../shaders/deflicker.spv");
        let reduction_spv = include_bytes!("../shaders/row_reduction.spv");

        let deflicker_shader = Self::create_shader_module(&device, deflicker_spv)?;
        let reduction_shader = Self::create_shader_module(&device, reduction_spv)?;

        // Descriptor set layouts
        // Deflicker layout: binding 0 = in (storage), binding 1 = gains (storage), binding 2 = out (storage)
        let deflicker_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let deflicker_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&deflicker_bindings);
        let deflicker_desc_layout = unsafe {
            device
                .create_descriptor_set_layout(&deflicker_layout_info, None)
                .map_err(|e| format!("vkCreateDescriptorSetLayout failed: {e}"))?
        };

        // Row reduction layout: binding 0 = in (storage), binding 1 = out means (storage)
        let reduction_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let reduction_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&reduction_bindings);
        let reduction_desc_layout = unsafe {
            device
                .create_descriptor_set_layout(&reduction_layout_info, None)
                .map_err(|e| format!("vkCreateDescriptorSetLayout failed: {e}"))?
        };

        // Pipeline layouts with push constants
        let deflicker_pc_range = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(16)]; // uint width, uint height, uint stride_words, float temporal_gain
        let deflicker_set_layouts = [deflicker_desc_layout];
        let deflicker_pipe_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&deflicker_set_layouts)
            .push_constant_ranges(&deflicker_pc_range);
        let deflicker_pipe_layout = unsafe {
            device
                .create_pipeline_layout(&deflicker_pipe_info, None)
                .map_err(|e| format!("vkCreatePipelineLayout failed: {e}"))?
        };

        let reduction_pc_range = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(12)]; // uint width, uint height, uint stride_words
        let reduction_set_layouts = [reduction_desc_layout];
        let reduction_pipe_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&reduction_set_layouts)
            .push_constant_ranges(&reduction_pc_range);
        let reduction_pipe_layout = unsafe {
            device
                .create_pipeline_layout(&reduction_pipe_info, None)
                .map_err(|e| format!("vkCreatePipelineLayout failed: {e}"))?
        };

        // Compute pipelines
        let main_name = CStr::from_bytes_with_nul(b"main\0").unwrap();

        let deflicker_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(deflicker_shader)
            .name(main_name);
        let deflicker_pipe_create = vk::ComputePipelineCreateInfo::default()
            .stage(deflicker_stage)
            .layout(deflicker_pipe_layout);
        let deflicker_pipeline = unsafe {
            device
                .create_compute_pipelines(vk::PipelineCache::null(), &[deflicker_pipe_create], None)
                .map_err(|(_, e)| format!("create_compute_pipelines deflicker failed: {e}"))?[0]
        };

        let reduction_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(reduction_shader)
            .name(main_name);
        let reduction_pipe_create = vk::ComputePipelineCreateInfo::default()
            .stage(reduction_stage)
            .layout(reduction_pipe_layout);
        let reduction_pipeline = unsafe {
            device
                .create_compute_pipelines(vk::PipelineCache::null(), &[reduction_pipe_create], None)
                .map_err(|(_, e)| format!("create_compute_pipelines reduction failed: {e}"))?[0]
        };

        // Descriptor pool
        let pool_sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(16)];
        let pool_create = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(8);
        let descriptor_pool = unsafe {
            device
                .create_descriptor_pool(&pool_create, None)
                .map_err(|e| format!("vkCreateDescriptorPool failed: {e}"))?
        };

        let layouts = [deflicker_desc_layout, reduction_desc_layout];
        let ds_alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);
        let sets = unsafe {
            device
                .allocate_descriptor_sets(&ds_alloc)
                .map_err(|e| format!("vkAllocateDescriptorSets failed: {e}"))?
        };
        let deflicker_desc_set = sets[0];
        let reduction_desc_set = sets[1];

        Ok(Self {
            _entry: entry,
            instance,
            device,
            _physical_device: physical_device,
            queue,
            _queue_family_index: queue_family_index,
            command_pool,
            command_buffer,
            deflicker_shader,
            reduction_shader,
            deflicker_desc_layout,
            deflicker_pipe_layout,
            deflicker_pipeline,
            reduction_desc_layout,
            reduction_pipe_layout,
            reduction_pipeline,
            descriptor_pool,
            deflicker_desc_set,
            reduction_desc_set,
            input_buffer: None,
            output_buffer: None,
            row_gains_buffer: None,
            reduction_buffer: None,
            memory_properties,
        })
    }

    fn create_shader_module(device: &ash::Device, bytes: &[u8]) -> Result<vk::ShaderModule, String> {
        if bytes.len() % 4 != 0 {
            return Err("SPIR-V byte length must be multiple of 4".into());
        }
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let info = vk::ShaderModuleCreateInfo::default().code(&words);
        unsafe {
            device
                .create_shader_module(&info, None)
                .map_err(|e| format!("vkCreateShaderModule failed: {e}"))
        }
    }

    fn ensure_buffers(
        &mut self,
        needed_frame_bytes: usize,
        needed_height: usize,
    ) -> Result<(), String> {
        let frame_buf_needed = (needed_frame_bytes + 63) & !63;
        let gains_buf_needed = needed_height * 4;

        let needs_realloc = self.input_buffer.as_ref().map_or(true, |b| (b.size as usize) < frame_buf_needed)
            || self.row_gains_buffer.as_ref().map_or(true, |b| (b.size as usize) < gains_buf_needed);

        if needs_realloc {
            if let Some(mut b) = self.input_buffer.take() { b.destroy(&self.device); }
            if let Some(mut b) = self.output_buffer.take() { b.destroy(&self.device); }
            if let Some(mut b) = self.row_gains_buffer.take() { b.destroy(&self.device); }
            if let Some(mut b) = self.reduction_buffer.take() { b.destroy(&self.device); }

            let in_buf = VulkanBuffer::new(&self.device, &self.memory_properties, frame_buf_needed, vk::BufferUsageFlags::STORAGE_BUFFER)?;
            let out_buf = VulkanBuffer::new(&self.device, &self.memory_properties, frame_buf_needed, vk::BufferUsageFlags::STORAGE_BUFFER)?;
            let gain_buf = VulkanBuffer::new(&self.device, &self.memory_properties, gains_buf_needed, vk::BufferUsageFlags::STORAGE_BUFFER)?;
            let red_buf = VulkanBuffer::new(&self.device, &self.memory_properties, gains_buf_needed, vk::BufferUsageFlags::STORAGE_BUFFER)?;

            // Update deflicker descriptor set
            let in_info = [vk::DescriptorBufferInfo::default().buffer(in_buf.buffer).offset(0).range(vk::WHOLE_SIZE)];
            let gain_info = [vk::DescriptorBufferInfo::default().buffer(gain_buf.buffer).offset(0).range(vk::WHOLE_SIZE)];
            let out_info = [vk::DescriptorBufferInfo::default().buffer(out_buf.buffer).offset(0).range(vk::WHOLE_SIZE)];

            let writes = [
                vk::WriteDescriptorSet::default()
                    .dst_set(self.deflicker_desc_set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&in_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(self.deflicker_desc_set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&gain_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(self.deflicker_desc_set)
                    .dst_binding(2)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&out_info),
            ];
            unsafe { self.device.update_descriptor_sets(&writes, &[]) };

            // Update reduction descriptor set
            let red_out_info = [vk::DescriptorBufferInfo::default().buffer(red_buf.buffer).offset(0).range(vk::WHOLE_SIZE)];
            let red_writes = [
                vk::WriteDescriptorSet::default()
                    .dst_set(self.reduction_desc_set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&in_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(self.reduction_desc_set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&red_out_info),
            ];
            unsafe { self.device.update_descriptor_sets(&red_writes, &[]) };

            self.input_buffer = Some(in_buf);
            self.output_buffer = Some(out_buf);
            self.row_gains_buffer = Some(gain_buf);
            self.reduction_buffer = Some(red_buf);
        }

        Ok(())
    }

    /// Computes row luminance means for a frame using the `row_reduction.comp` compute shader.
    /// Returns a vector of length `height` containing the normalized row means in [0.0, 1.0].
    pub fn compute_row_means(
        &mut self,
        src: &[u8],
        width: usize,
        height: usize,
        stride_bytes: usize,
    ) -> Result<Vec<f32>, String> {
        let stride_words = (stride_bytes + 3) / 4;
        let total_bytes = stride_words * height * 4;
        self.ensure_buffers(total_bytes, height)?;

        let in_buf = self.input_buffer.as_ref().unwrap();
        let red_buf = self.reduction_buffer.as_ref().unwrap();

        in_buf.write(&self.device, src);

        let pc_data = [
            width as u32,
            height as u32,
            stride_words as u32,
        ];
        let mut pc_bytes = [0u8; 12];
        for (i, val) in pc_data.iter().enumerate() {
            pc_bytes[i * 4..(i + 1) * 4].copy_from_slice(&val.to_le_bytes());
        }

        unsafe {
            let begin_info = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device
                .begin_command_buffer(self.command_buffer, &begin_info)
                .map_err(|e| format!("begin_command_buffer: {e}"))?;

            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.reduction_pipeline,
            );

            self.device.cmd_bind_descriptor_sets(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.reduction_pipe_layout,
                0,
                &[self.reduction_desc_set],
                &[],
            );

            self.device.cmd_push_constants(
                self.command_buffer,
                self.reduction_pipe_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                &pc_bytes,
            );

            self.device.cmd_dispatch(self.command_buffer, 1, height as u32, 1);

            self.device
                .end_command_buffer(self.command_buffer)
                .map_err(|e| format!("end_command_buffer: {e}"))?;

            let cbs = [self.command_buffer];
            let submit_info = [vk::SubmitInfo::default()
                .command_buffers(&cbs)];
            self.device
                .queue_submit(self.queue, &submit_info, vk::Fence::null())
                .map_err(|e| format!("queue_submit: {e}"))?;

            self.device
                .queue_wait_idle(self.queue)
                .map_err(|e| format!("queue_wait_idle: {e}"))?;
        }

        let mut out_bytes = vec![0u8; height * 4];
        red_buf.read(&self.device, &mut out_bytes);

        let mut out_means = vec![0.0f32; height];
        for (i, chunk) in out_bytes.chunks_exact(4).enumerate() {
            out_means[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }

        Ok(out_means)
    }

    /// Performs full-frame deflickering on GPU using `deflicker.comp`.
    /// Multiplies pixel (x, y) by `row_gains[y] * temporal_gain` and clamps to [0, 255].
    pub fn deflicker_frame(
        &mut self,
        src: &[u8],
        dst: &mut [u8],
        width: usize,
        height: usize,
        stride_bytes: usize,
        row_gains: &[f32],
        temporal_gain: f32,
    ) -> Result<(), String> {
        let stride_words = (stride_bytes + 3) / 4;
        let total_bytes = stride_words * height * 4;
        self.ensure_buffers(total_bytes, height)?;

        let in_buf = self.input_buffer.as_ref().unwrap();
        let out_buf = self.output_buffer.as_ref().unwrap();
        let gain_buf = self.row_gains_buffer.as_ref().unwrap();

        in_buf.write(&self.device, src);

        // Upload row gains
        let mut gain_bytes = vec![0u8; height * 4];
        for (i, &g) in row_gains.iter().take(height).enumerate() {
            gain_bytes[i * 4..(i + 1) * 4].copy_from_slice(&g.to_le_bytes());
        }
        gain_buf.write(&self.device, &gain_bytes);

        let pc_words = [
            width as u32,
            height as u32,
            stride_words as u32,
            temporal_gain.to_bits(),
        ];
        let mut pc_bytes = [0u8; 16];
        for (i, val) in pc_words.iter().enumerate() {
            pc_bytes[i * 4..(i + 1) * 4].copy_from_slice(&val.to_le_bytes());
        }

        let gx = (stride_words as u32 + 63) / 64;
        let gy = (height as u32 + 3) / 4;

        unsafe {
            let begin_info = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device
                .begin_command_buffer(self.command_buffer, &begin_info)
                .map_err(|e| format!("begin_command_buffer: {e}"))?;

            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.deflicker_pipeline,
            );

            self.device.cmd_bind_descriptor_sets(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.deflicker_pipe_layout,
                0,
                &[self.deflicker_desc_set],
                &[],
            );

            self.device.cmd_push_constants(
                self.command_buffer,
                self.deflicker_pipe_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                &pc_bytes,
            );

            self.device.cmd_dispatch(self.command_buffer, gx, gy, 1);

            self.device
                .end_command_buffer(self.command_buffer)
                .map_err(|e| format!("end_command_buffer: {e}"))?;

            let cbs = [self.command_buffer];
            let submit_info = [vk::SubmitInfo::default()
                .command_buffers(&cbs)];
            self.device
                .queue_submit(self.queue, &submit_info, vk::Fence::null())
                .map_err(|e| format!("queue_submit: {e}"))?;

            self.device
                .queue_wait_idle(self.queue)
                .map_err(|e| format!("queue_wait_idle: {e}"))?;
        }

        out_buf.read(&self.device, dst);
        Ok(())
    }
}

impl Drop for VulkanCompute {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();

            if let Some(mut b) = self.input_buffer.take() { b.destroy(&self.device); }
            if let Some(mut b) = self.output_buffer.take() { b.destroy(&self.device); }
            if let Some(mut b) = self.row_gains_buffer.take() { b.destroy(&self.device); }
            if let Some(mut b) = self.reduction_buffer.take() { b.destroy(&self.device); }

            self.device.destroy_descriptor_pool(self.descriptor_pool, None);

            self.device.destroy_pipeline(self.deflicker_pipeline, None);
            self.device.destroy_pipeline_layout(self.deflicker_pipe_layout, None);
            self.device.destroy_descriptor_set_layout(self.deflicker_desc_layout, None);

            self.device.destroy_pipeline(self.reduction_pipeline, None);
            self.device.destroy_pipeline_layout(self.reduction_pipe_layout, None);
            self.device.destroy_descriptor_set_layout(self.reduction_desc_layout, None);

            self.device.destroy_shader_module(self.deflicker_shader, None);
            self.device.destroy_shader_module(self.reduction_shader, None);

            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
