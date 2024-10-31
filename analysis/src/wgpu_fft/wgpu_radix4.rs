use num_complex::{Complex};
use rustfft::FftNum;
use std::num::NonZeroU64;
use wgpu::{util::DeviceExt, ComputePipeline, Device, Queue, ShaderModule};
use bytemuck::{Pod, Zeroable};

pub struct FFTCompute {
    device: Device,
    queue: Queue,
    pipeline: ComputePipeline,
}

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

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader1.wgsl"));
        
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            device,
            queue,
            pipeline,
        }
    }

    pub async fn compute_fft<T>(&self, data: &mut Vec<Complex<T>>)
    where 
        T: FftNum + Pod + Zeroable + Copy,
        Complex<T>: Pod + Zeroable,
    {
        // println!("1");
        let n = data.len();
        assert!(n.is_power_of_two(), "Input length must be a power of 2");
        let buffer_size = (data.len() * std::mem::size_of::<Complex<T>>()) as wgpu::BufferAddress;

        let storage_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Storage Buffer"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });

        // println!("2");

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FFT Bind Group"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: storage_buffer.as_entire_binding(),
            }],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("FFT Command Encoder"),
            });

            {
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("FFT Compute Pass"),
                    timestamp_writes: None,
                });
                compute_pass.set_pipeline(&self.pipeline);
                compute_pass.set_bind_group(0, Some(&bind_group), &[]);
                compute_pass.dispatch_workgroups(n as u32, 1, 1);  // 修改为数据长度
            }

        let result_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Result Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_buffer_to_buffer(&storage_buffer, 0, &result_buffer, 0, buffer_size);
        self.queue.submit(Some(encoder.finish()));

        let buffer_slice = result_buffer.slice(..);
        let (tx, rx) = futures::channel::oneshot::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.await.unwrap().unwrap();

        let mapped_range = buffer_slice.get_mapped_range();
        data.copy_from_slice(bytemuck::cast_slice(&mapped_range));
        // println!("3");
    }
}

// #[tokio::main]
// async fn main() {
//     let fft = FFTCompute::new().await;
   
//     let input_data = vec![
//         Complex32::new(1.0, 0.0),
//         Complex32::new(2.0, 0.0),
//         Complex32::new(3.0, 0.0),
//         Complex32::new(4.0, 0.0),
//         // Complex32::new(0.0, 0.0),
//         // Complex32::new(0.0, 0.0),
//         // Complex32::new(0.0, 0.0),
//         // Complex32::new(0.0, 0.0),        
//     ];

//     let result = fft.compute_fft(input_data).await;

//     println!("{:?}", result);
// }
