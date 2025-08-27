use bytemuck::{Pod, Zeroable};
use num_complex::{Complex, Complex32};
use rustfft::FftNum;
use wgpu::{ComputePipeline, Device, Queue};

pub struct FFTCompute {
    device: Device,
    queue: Queue,
    pipeline: ComputePipeline,
    output_buffer: wgpu::Buffer,
    result_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    buffer_size: wgpu::BufferAddress,
}

impl FFTCompute {
    pub async fn new(len: usize) -> Self {
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

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let n: usize = len;
        let buffer_size = (n * std::mem::size_of::<Complex32>()) as wgpu::BufferAddress;

        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let result_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Result Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FFT Bind Group"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: output_buffer.as_entire_binding(),
            }],
        });

        Self {
            device,
            queue,
            pipeline,
            output_buffer,
            result_buffer,
            bind_group,
            buffer_size,
        }
    }

    pub async fn compute_fft<T>(&self, data: &mut [Complex<T>])
    where
        T: FftNum + Pod + Zeroable + Copy,
        Complex<T>: Pod + Zeroable,
    {
        let n = data.len();
        assert!(n.is_power_of_two(), "Input length must be a power of 2");
        assert!(
            std::mem::size_of_val(data) as u64 <= self.buffer_size,
            "Input data too large for buffer"
        );

        self.queue
            .write_buffer(&self.output_buffer, 0, bytemuck::cast_slice(data));

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
            compute_pass.set_bind_group(0, Some(&self.bind_group), &[]);

            compute_pass.dispatch_workgroups(1, (n / 1024) as u32, 1);
        }

        encoder.copy_buffer_to_buffer(
            &self.output_buffer,
            0,
            &self.result_buffer,
            0,
            std::mem::size_of_val(data) as u64,
        );
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure_time;
    use rustfft::{FftPlanner, num_complex::Complex32};

    fn generate_random_data(len: usize) -> Vec<Complex32> {
        (0..len)
            .map(|_| {
                Complex32::new(
                    rand::random::<f32>() * 2.0 - 1.0,
                    rand::random::<f32>() * 2.0 - 1.0,
                )
            })
            .collect()
    }

    fn compare_complex_vectors(v1: &[Complex32], v2: &[Complex32], epsilon: f32) -> bool {
        if v1.len() != v2.len() {
            return false;
        }

        let mut max_diff_re = 0.0;
        let mut max_diff_im = 0.0;
        let mut max_diff_idx = 0;

        for (i, (a, b)) in v1.iter().zip(v2.iter()).enumerate() {
            let diff_re = (a.re - b.re).abs();
            let diff_im = (a.im - b.im).abs();

            if diff_re > max_diff_re {
                max_diff_re = diff_re;
                max_diff_idx = i;
            }
            if diff_im > max_diff_im {
                max_diff_im = diff_im;
            }
        }

        log::info!("Max real difference: {max_diff_re} at index {max_diff_idx}");
        log::info!("Max imaginary difference: {max_diff_im}");
        log::info!(
            "Value at max diff - v1: {}, v2: {}",
            v1[max_diff_idx],
            v2[max_diff_idx]
        );

        v1.iter()
            .zip(v2.iter())
            .all(|(a, b)| (a.re - b.re).abs() < epsilon && (a.im - b.im).abs() < epsilon)
    }

    async fn test(len: usize) {
        let mut planner = FftPlanner::<f32>::new();
        let cpu_fft = planner.plan_fft_forward(1024);
        let gpu_fft = FFTCompute::new(len).await;

        let mut cpu_data1 = generate_random_data(len);
        let mut cpu_data2 = cpu_data1.clone();
        let mut gpu_data1 = cpu_data1.clone();
        let mut gpu_data2 = gpu_data1.clone();

        measure_time!("CPU FFT1", cpu_fft.process(&mut cpu_data1));
        measure_time!("CPU FFT2", cpu_fft.process(&mut cpu_data2));
        measure_time!("GPU FFT1", gpu_fft.compute_fft(&mut gpu_data1).await);
        measure_time!("GPU FFT2", gpu_fft.compute_fft(&mut gpu_data2).await);

        const EPSILON: f32 = 1e-3;
        assert!(
            compare_complex_vectors(&cpu_data1, &gpu_data1, EPSILON),
            "CPU results differ more than expected"
        );
        assert!(
            compare_complex_vectors(&cpu_data2, &gpu_data2, EPSILON),
            "GPU results differ more than expected"
        );
    }

    #[tokio::test]
    async fn test_small_data() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        test(1024).await;
    }

    #[tokio::test]
    async fn test_big_data() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        test(1024 * 1024 * 8).await;
    }
}
