#[macro_export]
macro_rules! measure_time {
    ($func:expr) => {{
        let start = std::time::Instant::now();
        let result = $func;
        let duration = start.elapsed();
        log::info!("Time cost: {:?}", duration);
        result
    }};
    
    ($func:expr, $name:expr) => {{
        let start = std::time::Instant::now();
        let result = $func;
        let duration = start.elapsed();
        log::info!("{} Time cost: {:?}", $name, duration);
        result
    }};
}
