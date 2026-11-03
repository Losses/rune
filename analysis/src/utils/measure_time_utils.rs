#[macro_export]
macro_rules! measure_time {
    ($func:expr) => {{
        use std::time::Instant;
        let start = Instant::now();
        let result = $func;
        let duration = start.elapsed();
        log::debug!("Time cost: {:?}", {duration});
        result
    }};

    ($name:expr,$func:expr) => {{
        use std::time::Instant;
        let start = Instant::now();
        let result = $func;
        let duration = start.elapsed();
        log::debug!("{} time ost: {:?}", $name, duration);
        result
    }};

    ($($body:stmt)*) => {{
        use std::time::Instant;
        let start = Instant::now();
        let result = { $($body)* };
        let duration = start.elapsed();
        log::debug!("Time cost: {:?}", {duration});
        result
    }};

    ($name:expr, $($body:stmt)*) => {{
        use std::time::Instant;
        let start = Instant::now();
        let result = { $($body)* };
        let duration = start.elapsed();
        log::debug!("{} time cost: {:?}", $name, duration);
        result
    }};
}
