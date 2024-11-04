const pi: f32 = 3.1415926535897932384626433832795028841971;

// @group(0) @binding(0) var<storage, read> input_data: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read_write> output_data: array<vec2<f32>>;
// @group(0) @binding(2) var<storage, read_write> debug_data: array<vec2<f32>>;

var<workgroup> shared_data: array<vec2<f32>, 1024>;

fn complex_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        a.x * b.x - a.y * b.y,
        a.x * b.y + a.y * b.x
    );
}

fn twiddle(k: u32, N: u32) -> vec2<f32> {
    let angle = -2.0 * pi * f32(k) / f32(N);
    return vec2<f32>(cos(angle), sin(angle));
}

fn fft_compute(global_id1: u32) {
    let index = global_id1;
    let window_size = 1024u;
    let full_N = window_size;
    let N = full_N;

    // 每个线程需要处理四个数据点
    for (var i = 0u; i < 4u; i = i + 1u) {
        let local_index = index + i * 256u;  // 256是workgroup size
        if (local_index >= N) {
            continue;
        }

        var rev_index = 0u;
        var temp = local_index;
        let log4_N = u32(log2(f32(N)) / 2);  // 对于1024点，这里5

        for (var j = 0u; j < log4_N; j = j + 1u) {
            rev_index = rev_index << 2u;
            rev_index = rev_index | (temp & 3u);
            temp = temp >> 2u;
        }

        // 位反转置换
        if (local_index < rev_index && rev_index < N) {
            let temp1 = shared_data[local_index];
            shared_data[local_index] = shared_data[rev_index];
            shared_data[rev_index] = temp1;
        }
    }

    workgroupBarrier();  // 确保位反转完成

    // FFT 计算
    for (var step = 4u; step <= N; step = step << 2u) {
        let quarter_step = step >> 2u;

        // 每个线程处理四个蝶形运算
        for (var i = 0u; i < 4u; i = i + 1u) {
            let local_index = index + i * 256u;
            if (local_index >= N) {
                continue;
            }

            let pos = local_index % step;

            if (pos < quarter_step) {
                let base_idx = (local_index / step) * step;
                let k = pos;

                let x0 = shared_data[base_idx + k];
                let x1 = shared_data[base_idx + k + quarter_step];
                let x2 = shared_data[base_idx + k + 2u * quarter_step];
                let x3 = shared_data[base_idx + k + 3u * quarter_step];

                let W1 = twiddle(k, step);
                let W2 = twiddle(2u * k, step);
                let W3 = twiddle(3u * k, step);

                let t1 = complex_mul(x1, W1);
                let t2 = complex_mul(x2, W2);
                let t3 = complex_mul(x3, W3);

                let a0 = vec2<f32>(x0.x + t2.x, x0.y + t2.y);
                let a1 = vec2<f32>(x0.x - t2.x, x0.y - t2.y);
                let a2 = vec2<f32>(t1.x + t3.x, t1.y + t3.y);
                let a3 = vec2<f32>(t1.x - t3.x, t1.y - t3.y);

                shared_data[base_idx + k] = vec2<f32>(a0.x + a2.x, a0.y + a2.y);
                shared_data[base_idx + k + quarter_step] = vec2<f32>(a1.x + a3.y, a1.y - a3.x);
                shared_data[base_idx + k + 2u * quarter_step] = vec2<f32>(a0.x - a2.x, a0.y - a2.y);
                shared_data[base_idx + k + 3u * quarter_step] = vec2<f32>(a1.x - a3.y, a1.y + a3.x);
            }
        }

        workgroupBarrier();  // 确保每个步骤完成后同步
    }
}

fn copy_output_to_shared(local_id: u32, offset: u32) {
    for (var i = 0u; i < 4u; i = i + 1u) {
        let idx = local_id + i * 256u;
        if (idx < 1024u) {
            shared_data[idx] = output_data[idx + offset * 1024u];
        }
    }
}

fn copy_shared_to_output(local_id: u32, offset: u32) {
    for (var i = 0u; i < 4u; i = i + 1u) {
        let idx = local_id + i * 256u;
        if (idx < 1024u) {
            output_data[idx + offset * 1024u] = shared_data[idx];
        }
    }
}

@compute @workgroup_size(256, 1, 1)
fn main(
    @builtin(global_invocation_id) global_id: vec3<u32>,
) {
    copy_output_to_shared(global_id.x, global_id.y);

    workgroupBarrier();
    storageBarrier();

    fft_compute(global_id.x);

    workgroupBarrier();
    storageBarrier();

    copy_shared_to_output(global_id.x, global_id.y);
}