const pi: f32 = 3.141592653598979323846264338327950288;

@group(0) @binding(0)
var<storage, read_write> data: array<vec2<f32>>;

// 复数乘法辅助函数
fn complex_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        a.x * b.x - a.y * b.y,
        a.x * b.y + a.y * b.x
    );
}

// 计算旋转因子
fn twiddle(k: u32, N: u32) -> vec2<f32> {
    let angle = -2.0 * pi * f32(k) / f32(N);
    return vec2<f32>(cos(angle), sin(angle));
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    let N = arrayLength(&data);
    
    // 确保输入长度是4的幂
    if ((N & (N - 1u)) != 0u || N % 4u != 0u) {
        return;
    }

    // 比特反转（基4）
    var rev_index = 0u;
    var temp = index;
    let log4_N = u32(log2(f32(N)) / 2.0);
    
    for (var i = 0u; i < log4_N; i = i + 1u) {
        rev_index = rev_index << 2u;
        rev_index = rev_index | (temp & 3u);
        temp = temp >> 2u;
    }
    
    if (index < rev_index) {
        let temp = data[index];
        data[index] = data[rev_index];
        data[rev_index] = temp;
    }
    
    // Radix-4 蝶形运算
    for (var step = 4u; step <= N; step = step << 2u) {
        let group = index / step;
        let pos = index % step;
        let quarter_step = step >> 2u;
        
        if (pos < step) {
            let base_idx = group * step;
            let k = pos;
            
            if (k < quarter_step) {
                // 获取四个输入点
                let x0 = data[base_idx + k];
                let x1 = data[base_idx + k + quarter_step];
                let x2 = data[base_idx + k + 2u * quarter_step];
                let x3 = data[base_idx + k + 3u * quarter_step];
                
                // 计算旋转因子
                let W1 = twiddle(k, step);
                let W2 = twiddle(2u * k, step);
                let W3 = twiddle(3u * k, step);
                
                // 应用旋转因子
                let t1 = complex_mul(x1, W1);
                let t2 = complex_mul(x2, W2);
                let t3 = complex_mul(x3, W3);
                
                // 4点 FFT
                let a0 = vec2<f32>(x0.x + t2.x, x0.y + t2.y);
                let a1 = vec2<f32>(x0.x - t2.x, x0.y - t2.y);
                let a2 = vec2<f32>(t1.x + t3.x, t1.y + t3.y);
                let a3 = vec2<f32>(t1.x - t3.x, t1.y - t3.y);
                
                data[base_idx + k] = vec2<f32>(
                    a0.x + a2.x,
                    a0.y + a2.y
                );
                
                data[base_idx + k + quarter_step] = vec2<f32>(
                    a1.x + a3.y,
                    a1.y - a3.x
                );
                
                data[base_idx + k + 2u * quarter_step] = vec2<f32>(
                    a0.x - a2.x,
                    a0.y - a2.y
                );
                
                data[base_idx + k + 3u * quarter_step] = vec2<f32>(
                    a1.x - a3.y,
                    a1.y + a3.x
                );
            }
        }
    }
}