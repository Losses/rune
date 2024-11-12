#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(i32)]
pub enum ComputingDevice {
    Cpu = 0,
    Gpu = 1,
}

impl From<i32> for ComputingDevice {
    fn from(value: i32) -> Self {
        match value {
            0 => ComputingDevice::Cpu,
            1 => ComputingDevice::Gpu,
            _ => ComputingDevice::Gpu,
        }
    }
}

impl From<&str> for ComputingDevice {
    fn from(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "cpu" => ComputingDevice::Cpu,
            "gpu" => ComputingDevice::Gpu,
            _ => ComputingDevice::Gpu,
        }
    }
}
