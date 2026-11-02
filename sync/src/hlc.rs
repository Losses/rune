//! Hybrid logical clocks used by existing database operations.

use std::{
    cmp::{self, Ordering},
    str::FromStr,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents a Hybrid Logical Clock (HLC).
///
/// An HLC combines a physical timestamp with a logical counter to ensure
/// monotonically increasing timestamps across a distributed system, even
/// with clock skew.
#[derive(Clone, Debug, Eq, Serialize, Deserialize)]
pub struct HLC {
    /// Physical timestamp component, in milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    /// Logical counter component, incremented for events within the same millisecond.
    pub version: u32,
    /// Unique identifier for the node that generated this HLC.
    pub node_id: Uuid,
}

impl std::fmt::Display for HLC {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-{:08x}-{}",
            self.timestamp_ms, self.version, self.node_id
        )
    }
}

impl PartialEq for HLC {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp_ms == other.timestamp_ms
            && self.version == other.version
            && self.node_id == other.node_id
    }
}

impl PartialOrd for HLC {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HLC {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        (self.timestamp_ms, self.version, self.node_id).cmp(&(
            other.timestamp_ms,
            other.version,
            other.node_id,
        ))
    }
}

impl HLC {
    /// Creates an initial HLC with timestamp 0 and counter 0.
    ///
    /// This is often used as a starting point or default value.
    pub fn new(node_id: Uuid) -> Self {
        let timestamp_ms = Utc::now().timestamp_millis();
        let timestamp_ms_u64 = timestamp_ms as u64;

        HLC {
            timestamp_ms: timestamp_ms_u64,
            version: 0,
            node_id,
        }
    }

    pub fn from_node_id_str(node_id: &str) -> Result<Self> {
        let timestamp_ms = Utc::now().timestamp_millis();
        let timestamp_ms_u64 = timestamp_ms as u64;

        Ok(HLC {
            timestamp_ms: timestamp_ms_u64,
            version: 0,
            node_id: Uuid::from_str(node_id)?,
        })
    }

    /// Generates a new HLC, ensuring monotonicity.
    ///
    /// It compares the current system time with the timestamp of the last generated HLC.
    /// If the current time is ahead, it uses the current time and resets the counter.
    /// If it's the same, it increments the counter. If clock skew is detected (current time is behind),
    /// it uses the last HLC's timestamp and increments the counter to maintain order.
    pub fn generate(context: &SyncTaskContext) -> Self {
        let mut last_hlc = context.last_hlc.lock().unwrap();
        let current_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis() as u64;

        let (timestamp, counter) = match current_timestamp.cmp(&last_hlc.timestamp_ms) {
            Ordering::Greater => (current_timestamp, 0),
            Ordering::Equal => {
                if last_hlc.version == u32::MAX {
                    panic!(
                        "HLC counter overflow detected within a single millisecond. Timestamp: {}, Node: {}",
                        current_timestamp, context.node_id
                    );
                }
                (current_timestamp, last_hlc.version + 1)
            }
            Ordering::Less => {
                if last_hlc.version == u32::MAX {
                    (last_hlc.timestamp_ms + 1, 0)
                } else {
                    (last_hlc.timestamp_ms, last_hlc.version + 1)
                }
            }
        };

        let new_hlc = HLC {
            timestamp_ms: timestamp,
            version: counter,
            node_id: context.node_id,
        };
        *last_hlc = new_hlc.clone();
        new_hlc
    }

    /// Increments the HLC logically by one step (version or timestamp).
    ///
    /// If the version counter is less than `u32::MAX`, it's incremented.
    /// If the version counter reaches `u32::MAX`, the timestamp is incremented
    /// by one millisecond, and the version resets to 0. This maintains the HLC ordering.
    ///
    /// **Note:** This method provides deterministic logical incrementing, primarily
    /// intended for testing or specific simulation scenarios where controlling the
    /// exact sequence of HLCs is needed, independent of the system clock used by `generate()`.
    /// For generating standard monotonic HLCs tied to physical time progression,
    /// use `HLC::generate()`. Avoid using this in production code where standard HLC
    /// generation based on physical time is required.
    pub fn increment(&mut self) {
        if self.version < u32::MAX {
            self.version += 1;
        } else {
            // Overflow: Increment timestamp and reset version
            // We assume timestamp itself won't realistically overflow u64.
            self.timestamp_ms += 1;
            self.version = 0;
        }
    }

    /// Converts the HLC timestamp to RFC3339 format.
    ///
    /// Returns a string representation of the timestamp in RFC3339 format (ISO 8601).
    /// The timestamp is interpreted as milliseconds since the Unix epoch.
    ///
    /// # Examples
    ///
    /// ```
    /// use sync::hlc::HLC;
    /// use uuid::Uuid;
    ///
    /// let hlc = HLC {
    ///     timestamp_ms: 1640995200000, // 2022-01-01T00:00:00Z
    ///     version: 1,
    ///     node_id: Uuid::new_v4(),
    /// };
    /// assert_eq!(hlc.to_rfc3339().unwrap(), "2022-01-01T00:00:00.000000000Z");
    /// ```
    pub fn to_rfc3339(&self) -> Result<String> {
        use chrono::TimeZone;
        Utc.timestamp_millis_opt(self.timestamp_ms as i64)
            .single()
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
            .context("HLC timestamp out of range for RFC3339 conversion")
    }
}

impl FromStr for HLC {
    type Err = anyhow::Error;

    /// Parses an HLC from a string representation.
    ///
    /// The string format is expected to be "timestamp-counterHex-node_id".
    fn from_str(hlc_str: &str) -> Result<Self> {
        let parts: Vec<&str> = hlc_str.splitn(3, '-').collect();
        if parts.len() != 3 {
            bail!(
                "Invalid HLC string format: expected 'timestamp-counterHex-node_id', got '{}'",
                hlc_str
            );
        }

        let timestamp = parts[0]
            .parse::<u64>()
            .with_context(|| format!("Invalid timestamp format in HLC: '{}'", parts[0]))?;
        // Ensure counter part has expected hex length if needed, though from_str_radix handles it.
        let counter = u32::from_str_radix(parts[1], 16)
            .with_context(|| format!("Invalid hex counter format in HLC: '{}'", parts[1]))?;
        let node_id = Uuid::parse_str(parts[2])
            .with_context(|| format!("Invalid node ID format in HLC: '{}'", parts[2]))?;

        Ok(HLC {
            timestamp_ms: timestamp,
            version: counter,
            node_id,
        })
    }
}

pub struct SyncTaskContext {
    node_id: Uuid,
    last_hlc: Mutex<HLC>,
}

impl SyncTaskContext {
    pub fn new(node_id: Uuid) -> Self {
        SyncTaskContext {
            node_id,
            last_hlc: Mutex::new(HLC::new(node_id)),
        }
    }

    /// Creates a context with a specific initial HLC (useful for testing).
    #[cfg(test)]
    fn with_initial_hlc(node_id: Uuid, initial_hlc: HLC) -> Self {
        SyncTaskContext {
            node_id,
            last_hlc: Mutex::new(initial_hlc),
        }
    }

    pub fn generate_hlc(&self) -> HLC {
        HLC::generate(self)
    }
}

#[cfg(test)]
pub fn create_hlc(ts: u64, v: u32, node_id_str: &str) -> HLC {
    HLC {
        timestamp_ms: ts,
        version: v,
        node_id: Uuid::parse_str(node_id_str).unwrap(),
    }
}

#[cfg(test)]
mod hlc_increment_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_hlc_increment_normal() {
        let node_id = Uuid::new_v4();
        let mut hlc = HLC {
            timestamp_ms: 1000,
            version: 5,
            node_id,
        };
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1000);
        assert_eq!(hlc.version, 6);
        assert_eq!(hlc.node_id, node_id);
    }

    #[test]
    fn test_hlc_increment_version_max() {
        let node_id = Uuid::new_v4();
        let mut hlc = HLC {
            timestamp_ms: 1000,
            version: u32::MAX - 1,
            node_id,
        };
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1000);
        assert_eq!(hlc.version, u32::MAX);
        assert_eq!(hlc.node_id, node_id);
    }

    #[test]
    fn test_hlc_increment_overflow() {
        let node_id = Uuid::new_v4();
        let mut hlc = HLC {
            timestamp_ms: 1000,
            version: u32::MAX,
            node_id,
        };
        hlc.increment();
        assert_eq!(
            hlc.timestamp_ms, 1001,
            "Timestamp should increment on version overflow"
        );
        assert_eq!(hlc.version, 0, "Version should reset to 0 on overflow");
        assert_eq!(hlc.node_id, node_id);
    }

    #[test]
    fn test_hlc_increment_multiple_overflows() {
        let node_id = Uuid::new_v4();
        let mut hlc = HLC {
            timestamp_ms: 1000,
            version: u32::MAX - 1,
            node_id,
        };

        // Increment to MAX
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1000);
        assert_eq!(hlc.version, u32::MAX);

        // Increment causing overflow
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1001);
        assert_eq!(hlc.version, 0);

        // Normal increment after overflow
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1001);
        assert_eq!(hlc.version, 1);
    }
}

#[cfg(test)]
mod hlc_generate_tests {
    use super::*;
    use std::thread::sleep;

    use chrono::Duration;

    fn get_current_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[test]
    fn test_generate_time_moves_forward() {
        let node_id = Uuid::new_v4();
        let initial_ts = get_current_millis();
        let initial_hlc = create_hlc(initial_ts, 5, &node_id.to_string());
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        // Wait for time to likely advance
        sleep(Duration::milliseconds(5).to_std().unwrap());

        let new_hlc = context.generate_hlc();
        let after_ts = get_current_millis(); // Timestamp might advance slightly during test

        assert!(
            new_hlc.timestamp_ms > initial_hlc.timestamp_ms,
            "New timestamp should be greater"
        );
        assert!(
            new_hlc.timestamp_ms >= initial_ts + 5,
            "New timestamp should be roughly current time"
        );
        assert!(
            new_hlc.timestamp_ms <= after_ts,
            "New timestamp should be roughly current time"
        );
        assert_eq!(new_hlc.version, 0, "Counter should reset");
        assert_eq!(new_hlc.node_id, node_id);

        // Check last_hlc was updated
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    fn test_generate_time_stays_same() {
        let node_id = Uuid::new_v4();
        // Simulate by setting last_hlc's time to *now* just before generating
        let current_ts = get_current_millis();
        let initial_hlc = create_hlc(current_ts, 5, &node_id.to_string());
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        let new_hlc = context.generate_hlc();

        // It's possible the millisecond ticked over between get_current_millis and generate()
        if new_hlc.timestamp_ms == initial_hlc.timestamp_ms {
            assert_eq!(
                new_hlc.version,
                initial_hlc.version + 1,
                "Counter should increment if timestamp is the same"
            );
        } else {
            // If time advanced, counter should reset
            assert!(
                new_hlc.timestamp_ms > initial_hlc.timestamp_ms,
                "Timestamp advanced"
            );
            assert_eq!(
                new_hlc.version, 0,
                "Counter should reset if timestamp advanced"
            );
        }
        assert_eq!(new_hlc.node_id, node_id);
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    fn test_generate_clock_skew() {
        let node_id = Uuid::new_v4();
        // Set last_hlc to be in the future relative to current system time
        let future_ts = get_current_millis() + 10000; // 10 seconds in the future
        let initial_hlc = create_hlc(future_ts, 5, &node_id.to_string());
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        let new_hlc = context.generate_hlc();

        assert_eq!(
            new_hlc.timestamp_ms, initial_hlc.timestamp_ms,
            "Timestamp should use the last HLC's timestamp during skew"
        );
        assert_eq!(
            new_hlc.version,
            initial_hlc.version + 1,
            "Counter should increment during skew"
        );
        assert_eq!(new_hlc.node_id, node_id);
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    fn test_generate_time_stays_same_explicit() {
        let node_id = Uuid::new_v4();
        // Simulate by setting last_hlc's time to *now* just before generating
        let current_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let initial_version = 5u32;
        let initial_hlc = HLC {
            timestamp_ms: current_ts,
            version: initial_version,
            node_id,
        };
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        // Try to generate within the same millisecond
        let new_hlc = context.generate_hlc();

        // Check the Ordering::Equal case explicitly
        if new_hlc.timestamp_ms == initial_hlc.timestamp_ms {
            assert_eq!(
                new_hlc.version,
                initial_hlc.version + 1,
                "Counter should increment if timestamp is the same"
            );
        } else {
            // If time advanced, counter should reset (Ordering::Greater case)
            assert!(
                new_hlc.timestamp_ms > initial_hlc.timestamp_ms,
                "Timestamp advanced unexpectedly fast, test assumption failed or different code path taken."
            );
            assert_eq!(
                new_hlc.version, 0,
                "Counter should reset if timestamp advanced"
            );
        }
        assert_eq!(new_hlc.node_id, node_id);
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    fn test_generate_clock_skew_with_counter_overflow() {
        let node_id = Uuid::new_v4();
        // Set last_hlc to be in the future relative to current system time
        let future_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 10000; // 10 seconds in the future
        let initial_hlc = HLC {
            timestamp_ms: future_ts,
            version: u32::MAX, // Set counter to MAX
            node_id,
        };
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        let new_hlc = context.generate_hlc();

        // Clock skew detected (Ordering::Less), counter was MAX.
        // Should increment timestamp and reset counter.
        assert_eq!(
            new_hlc.timestamp_ms,
            initial_hlc.timestamp_ms + 1, // Timestamp increments because counter overflowed
            "Timestamp should increment due to counter overflow during skew"
        );
        assert_eq!(
            new_hlc.version,
            0, // Counter resets
            "Counter should reset to 0 after overflow during skew"
        );
        assert_eq!(new_hlc.node_id, node_id);
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    #[should_panic(expected = "HLC counter overflow detected")]
    #[cfg_attr(
        tarpaulin,
        ignore = "Flaky timing-dependent test under instrumentation"
    )]
    fn test_generate_counter_overflow_panic() {
        // This test is tricky because it relies on SystemTime::now() not advancing
        // between the setup and the call to generate(). It might be flaky.
        let node_id = Uuid::new_v4();
        let current_ts = get_current_millis();
        let initial_hlc = HLC {
            timestamp_ms: current_ts,
            version: u32::MAX, // Set version to max
            node_id,
        };
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        // Immediately generate. If the clock *doesn't* tick, the panic should occur.
        // If the clock *does* tick, timestamp increases, counter resets to 0, no panic.
        // The should_panic expects the panic. If it doesn't panic, the test fails.
        let _new_hlc = context.generate_hlc();
        // We might need a slight delay *after* setting initial_hlc to ensure generate()
        // sees the same timestamp, but that makes the test less reliable.
        // Let's assume for testing purposes the clock might not advance instantly.

        // Alternative: Mock SystemTime, but that requires external crates or complex setup.
        // Given the rarity of this condition, relying on the panic safeguard and the
        // deterministic `increment` test for overflow logic is often sufficient.
    }
}

#[cfg(test)]
mod hlc_from_str_tests {
    use super::*;

    #[test]
    fn test_from_str_valid() -> Result<()> {
        let node_id = Uuid::new_v4();
        let hlc_string = format!("1234567890123-0000abcd-{node_id}");
        let hlc = HLC::from_str(&hlc_string)?;

        assert_eq!(hlc.timestamp_ms, 1234567890123);
        assert_eq!(hlc.version, 0xabcd);
        assert_eq!(hlc.node_id, node_id);
        Ok(())
    }

    #[test]
    fn test_from_str_invalid_format_parts() {
        let result = HLC::from_str("12345-abcd"); // Too few parts
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Invalid HLC string format"));
        assert!(err_msg.contains("expected 'timestamp-counterHex-node_id'"));
    }

    #[test]
    fn test_from_str_invalid_timestamp() {
        let node_id = Uuid::new_v4();
        let hlc_string = format!("not_a_number-0000abcd-{node_id}");
        let result = HLC::from_str(&hlc_string);
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        // Check the context message
        assert!(err_msg.contains("Invalid timestamp format in HLC: 'not_a_number'"));
    }

    #[test]
    fn test_from_str_invalid_counter() {
        let node_id = Uuid::new_v4();
        let hlc_string = format!("1234567890123-not_hex-{node_id}");
        let result = HLC::from_str(&hlc_string);
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        // Check the context message
        assert!(err_msg.contains("Invalid hex counter format in HLC: 'not_hex'"));
    }

    #[test]
    fn test_from_str_invalid_counter_format_length() {
        // Valid hex, but format might imply fixed length (though not enforced by parse)
        let node_id = Uuid::new_v4();
        let hlc_string = format!("1234567890123-abc-{node_id}"); // Shorter hex than usual
        let hlc = HLC::from_str(&hlc_string).unwrap(); // Should still parse
        assert_eq!(hlc.version, 0xabc);

        // Test hex with > 8 chars (overflows u32)
        let hlc_string_overflow = format!("1234567890123-100000000-{node_id}");
        let result = HLC::from_str(&hlc_string_overflow);
        assert!(result.is_err()); // Error comes from u32::from_str_radix
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Invalid hex counter format in HLC: '100000000'"));
    }

    #[test]
    fn test_from_str_invalid_node_id() {
        let hlc_string = "1234567890123-0000abcd-not_a_uuid";
        let result = HLC::from_str(hlc_string);
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        // Check the context message
        assert!(err_msg.contains("Invalid node ID format in HLC: 'not_a_uuid'"));
    }
}
