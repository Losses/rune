use num_complex::{Complex, Complex32};
use rustfft::FftNum;
use std::{num::NonZeroU64, time::Instant};
use wgpu::{util::DeviceExt, ComputePipeline, Device, Queue, ShaderModule};
use bytemuck::{Pod, Zeroable};

pub struct FFTCompute {
    device: Device,
    queue: Queue,
    pipeline: ComputePipeline,
    input_buffer: wgpu::Buffer,
    output_buffer: wgpu::Buffer,
    debug_buffer: wgpu::Buffer,
    result_buffer: wgpu::Buffer,
    result_buffer1: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    buffer_size: wgpu::BufferAddress,
}

const LEN: usize = 1024 * 1024 * 8;
const A: usize = 1024;

impl FFTCompute {
    pub async fn new() -> Self {
        let instance = wgpu::Instance::default();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                },
                None,
            )
            .await
            .unwrap();

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader7.wgsl"));
        
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let n: usize = LEN;
        let buffer_size = (n * std::mem::size_of::<Complex32>()) as wgpu::BufferAddress;

        let input_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Input Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let debug_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let result_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Result Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let result_buffer1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Result1 Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FFT Bind Group"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                // wgpu::BindGroupEntry {
                //     binding: 0,
                //     resource: input_buffer.as_entire_binding(),
                // },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output_buffer.as_entire_binding(),
                },
                // wgpu::BindGroupEntry {
                //     binding: 2,
                //     resource: debug_buffer.as_entire_binding(),
                // },
            ],
        });

        Self {
            device,
            queue,
            pipeline,
            input_buffer,
            output_buffer,
            debug_buffer,
            result_buffer,
            result_buffer1,
            bind_group,
            buffer_size,
        }
    }

    pub async fn compute_fft<T>(&self, data: &mut Vec<Complex<T>>) -> Vec<Complex<T>>
    where 
        T: FftNum + Pod + Zeroable + Copy,
        Complex<T>: Pod + Zeroable,
    {
        let start_total = Instant::now();
        let n = data.len();
        assert!(n.is_power_of_two(), "Input length must be a power of 2");
        assert!((n * std::mem::size_of::<Complex<T>>()) as u64 <= self.buffer_size, "Input data too large for buffer");

        // 将输入数据复制到输入缓冲区
        self.queue.write_buffer(&self.input_buffer, 0, bytemuck::cast_slice(data));

        // 清空输出缓冲区
        let zero_buffer = vec![0u8; self.buffer_size as usize];
        self.queue.write_buffer(&self.output_buffer, 0, bytemuck::cast_slice(data));

        let zero_buffer = vec![0u8; self.buffer_size as usize];
        self.queue.write_buffer(&self.debug_buffer, 0, &zero_buffer);

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("FFT Command Encoder"),
        });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("FFT Compute Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.pipeline);
            compute_pass.set_bind_group(0, Some(&self.bind_group), &[]);
            
            let workgroup_size = 1024;
            let total_workgroups = 2;
            
            // compute_pass.dispatch_workgroups(256, 1, 1);
            compute_pass.dispatch_workgroups(1, (LEN / 1024) as u32, 1);

            // compute_pass.dispatch_workgroups(256, 1, 1);
            // let max_workgroups = 65535;
            // let num_dispatches = (total_workgroups + max_workgroups - 1) / max_workgroups;
            
            // for i in 0..num_dispatches {
            //     let remaining_workgroups = total_workgroups - i * max_workgroups;
            //     let current_workgroups = remaining_workgroups.min(max_workgroups);
            //     compute_pass.dispatch_workgroups(current_workgroups as u32, 1, 1);
            // }
        }

        encoder.copy_buffer_to_buffer(&self.output_buffer, 0, &self.result_buffer, 0, (n * std::mem::size_of::<Complex<T>>()) as u64);
        encoder.copy_buffer_to_buffer(&self.debug_buffer, 0, &self.result_buffer1, 0, (n * std::mem::size_of::<Complex<T>>()) as u64);
        self.queue.submit(Some(encoder.finish()));

        let buffer_slice = self.result_buffer.slice(..);
        let (tx, rx) = futures::channel::oneshot::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });

        self.device.poll(wgpu::Maintain::Wait);

        rx.await.unwrap().unwrap();

        let mapped_range = buffer_slice.get_mapped_range();
        data.copy_from_slice(bytemuck::cast_slice(&mapped_range));

        drop(mapped_range);  
        self.result_buffer.unmap(); 

        let debug_buffer_slice = self.result_buffer1.slice(..);
        let (tx1, rx1) = futures::channel::oneshot::channel();
        debug_buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx1.send(result).unwrap();
        });

        self.device.poll(wgpu::Maintain::Wait);
        rx1.await.unwrap().unwrap();

        let mapped_range1 = debug_buffer_slice.get_mapped_range();
        let debug_data: Vec<Complex<T>> = bytemuck::cast_slice(&mapped_range1).to_vec();

        drop(mapped_range1);
        self.result_buffer1.unmap();

        debug_data
    }
}