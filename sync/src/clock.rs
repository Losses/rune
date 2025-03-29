//! Implementation of Cristian's Algorithm for clock synchronization in Rust.
//!
//! This library provides a basic implementation of Cristian's Algorithm for synchronizing
//! a local clock with a reference time source over a network. It focuses on the core
//! algorithm and data structures, leaving the actual network communication to the user.
//!
//! # Features
//!
//! * **Cristian's Algorithm Core:** Implements the fundamental logic of Cristian's algorithm.
//! * **Multiple Sampling:** Supports taking multiple time samples to improve accuracy and
//!   mitigate network jitter and outliers.
//! * **Median Offset Calculation:** Uses the median of the sampled offsets to provide a robust
//!   estimate of the clock offset, resistant to occasional erroneous samples.
//! * **Separation of Concerns:**  Designed to be communication-agnostic. Users provide a
//!   `TimestampExchanger` implementation to handle network communication, allowing
//!   flexibility in choosing network protocols (e.g., UDP, TCP, custom protocols).
//! * **Initial Synchronization:** Provides a function `initial_sync` for initial clock calibration.
//! * **Offset Checking:** Provides a function `check_offset` for checking the current offset
//!   during task execution, useful for drift correction.
//! * **Error Handling:** Uses a custom `CristianError` enum to represent potential errors
//!   during the synchronization process.
//! * **Well-documented:** Includes detailed English comments explaining the algorithm,
//!   data structures, and functions.
//! * **Testable:** Includes unit tests to verify the correctness of the algorithm.
//!
//! # Usage
//!
//! 1. **Define a `TimestampExchanger`:** Implement the `TimestampExchanger` trait to handle
//!    the communication with your reference time source. This involves sending a timestamp
//!    request and receiving a timestamp response over your chosen network protocol.
//! 2. **Configure `CristianConfig`:** Create a `CristianConfig` instance to specify the
//!    number of samples for initial synchronization.
//! 3. **Perform Initial Synchronization:** Call the `initial_sync` function, passing your
//!    `CristianConfig` and `TimestampExchanger` instance. This will return a `ClockOffset`
//!    representing the calculated clock offset.
//! 4. **Check Offset During Execution (Optional):** Periodically call the `check_offset` function
//!    with your `TimestampExchanger` to get the current clock offset for drift correction.
//!
//! # Example (Illustrative - Requires Network Implementation)
//!
//! ```rust,no_run
//! use std::time::{Instant, Duration};
//! use std::thread;
//!
//! use clock::{CristianConfig, CristianError, initial_sync, TimestampExchanger, Timestamp, check_offset};
//!
//! //  Placeholder: Replace with your actual network implementation
//! struct MyNetworkExchanger;
//!
//! impl TimestampExchanger for MyNetworkExchanger {
//!     fn exchange_timestamp(&self) -> Result<Timestamp, CristianError> {
//!         //  Implement network communication here to get timestamp from server
//!         // Example: (This is just a placeholder - network code is needed)
//!         thread::sleep(Duration::from_millis(20)); // Simulate network delay
//!         let server_timestamp: Timestamp = Instant::now().elapsed().as_nanos() as Timestamp + 5000; // Simulate server time
//!         // Replace above with actual network request/response to get server timestamp
//!         Ok(server_timestamp)
//!     }
//! }
//! // End Placeholder
//!
//! fn main() -> Result<(), CristianError> {
//!     let config = CristianConfig::default(); // Use default config (5 samples)
//!     let network_exchanger = MyNetworkExchanger; // Use your network implementation
//!
//!     // Perform initial synchronization
//!     let initial_offset = initial_sync(config, &network_exchanger)?;
//!     println!("Initial clock offset: {} ns", initial_offset);
//!     // Apply this offset to your local clock if needed.
//!
//!     // Example: Check offset during task execution (periodically)
//!     let current_offset = check_offset(&network_exchanger)?;
//!     println!("Current clock offset: {} ns", current_offset);
//!
//!     Ok(())
//! }
//! ```
//!
//! # Note
//!
//! * This implementation focuses on the algorithm itself. You need to implement the
//!   `TimestampExchanger` trait with your specific network communication logic to make it
//!   functional in a real-world scenario.
//! * The accuracy of Cristian's Algorithm depends on the network latency and its symmetry.
//!   If the network latency is high or asymmetric, the accuracy may be reduced.
//! * For more advanced synchronization needs, consider using more sophisticated algorithms
//!   like NTP (Network Time Protocol) or PTP (Precision Time Protocol).
//!

use std::time::Instant;

/// Represents a timestamp in nanoseconds since an epoch (e.g., system start).
/// You can adjust the underlying type as needed (e.g., `i64` for nanoseconds since Unix epoch).
pub type Timestamp = i64;

/// Represents a clock offset in nanoseconds.
pub type ClockOffset = i64;

/// Configuration for Cristian's Algorithm.
#[derive(Debug, Clone, Copy)]
pub struct CristianConfig {
    /// Number of samples to collect for calculating the clock offset.
    pub num_samples: u32,
}

impl Default for CristianConfig {
    fn default() -> Self {
        CristianConfig {
            num_samples: 5, // Default number of samples
        }
    }
}

/// Errors that can occur during Cristian's algorithm execution.
#[derive(Debug, thiserror::Error)]
pub enum CristianError {
    /// Error during timestamp exchange with the reference node.
    #[error("Timestamp exchange failed: {0}")]
    ExchangeError(String),
    /// No valid timestamps received to calculate offset.
    #[error("No valid timestamps received")]
    NoValidTimestamps,
}

/// Trait defining the timestamp exchange mechanism.
/// Users need to implement this trait to provide the communication layer.
pub trait TimestampExchanger {
    /// Exchanges timestamp with the reference node.
    ///
    /// This function should:
    /// 1. Send a timestamp request to the reference node.
    /// 2. Receive the timestamp from the reference node.
    ///
    /// Returns:
    /// - `Ok(Timestamp)`: The timestamp received from the reference node.
    /// - `Err(CristianError)`: If the exchange fails.
    fn exchange_timestamp(&self) -> Result<Timestamp, CristianError>;
}

/// Executes Cristian's Algorithm to perform initial clock synchronization.
///
/// This function performs multiple timestamp exchanges with the reference node,
/// calculates the clock offset for each sample, and returns the median offset
/// as the final clock offset.
///
/// # Arguments
///
/// * `config`: Configuration parameters for Cristian's Algorithm.
/// * `timestamp_exchanger`: An implementation of `TimestampExchanger` trait
///                          that handles the communication with the reference node.
///
/// # Returns
///
/// * `Ok(ClockOffset)`: The calculated median clock offset in nanoseconds.
/// * `Err(CristianError)`: If synchronization fails due to communication errors
///                          or lack of valid timestamps.
///
/// # Example
///
/// ```rust,no_run
/// # use cristian_algorithm::{CristianConfig, CristianError, initial_sync, TimestampExchanger, Timestamp};
/// # use std::time::{Instant, Duration};
/// # use std::thread;
/// #
/// // Mock TimestampExchanger for example purposes (replace with your actual implementation)
/// struct MockExchanger;
///
/// impl TimestampExchanger for MockExchanger {
///     fn exchange_timestamp(&self) -> Result<Timestamp, CristianError> {
///         // Simulate network delay and server timestamp retrieval
///         thread::sleep(Duration::from_millis(50)); // Simulate network delay
///         let server_timestamp = Instant::now().elapsed().as_nanos() as Timestamp + 1000; // Simulate server time ahead
///         Ok(server_timestamp)
///     }
/// }
///
/// fn main() -> Result<(), CristianError> {
///     let config = CristianConfig::default();
///     let exchanger = MockExchanger; // Replace with your actual TimestampExchanger
///     let offset = initial_sync(config, &exchanger)?;
///     println!("Calculated clock offset: {} ns", offset);
///     Ok(())
/// }
/// ```
pub fn initial_sync<T: TimestampExchanger>(
    config: CristianConfig,
    timestamp_exchanger: &T,
) -> Result<ClockOffset, CristianError> {
    let mut offsets: Vec<ClockOffset> = Vec::new();
    let mut valid_samples = 0;

    for _ in 0..config.num_samples {
        match sample_offset(timestamp_exchanger) {
            Ok(offset) => {
                offsets.push(offset);
                valid_samples += 1;
            }
            Err(e) => {
                eprintln!("Error during timestamp sampling: {}", e);
                // Continue to next sample, but log the error.
            }
        }
    }

    if valid_samples == 0 {
        return Err(CristianError::NoValidTimestamps);
    }

    calculate_median_offset(&offsets)
}

/// Checks the clock offset in a task execution context using Cristian's Algorithm (single sample).
///
/// This function performs a single timestamp exchange with the reference node
/// and calculates the clock offset. This is useful for periodically checking
/// and correcting clock drift during task execution.
///
/// # Arguments
///
/// * `timestamp_exchanger`: An implementation of `TimestampExchanger` trait
///                          that handles the communication with the reference node.
///
/// # Returns
///
/// * `Ok(ClockOffset)`: The calculated clock offset in nanoseconds for this sample.
/// * `Err(CristianError)`: If the offset check fails due to communication errors.
///
/// # Example
///
/// ```rust,no_run
/// # use cristian_algorithm::{CristianError, check_offset, TimestampExchanger, Timestamp};
/// # use std::time::{Instant, Duration};
/// # use std::thread;
/// #
/// // Mock TimestampExchanger (replace with your actual implementation)
/// # struct MockExchanger;
/// #
/// # impl TimestampExchanger for MockExchanger {
/// #    fn exchange_timestamp(&self) -> Result<Timestamp, CristianError> {
/// #        thread::sleep(Duration::from_millis(50));
/// #        let server_timestamp = Instant::now().elapsed().as_nanos() as Timestamp + 1000;
/// #        Ok(server_timestamp)
/// #    }
/// # }
/// #
/// # fn main() -> Result<(), CristianError> {
/// #    let exchanger = MockExchanger; // Replace with your actual TimestampExchanger
///     let current_offset = check_offset(&exchanger)?;
///     println!("Current clock offset: {} ns", current_offset);
/// #    Ok(())
/// # }
/// ```
pub fn check_offset<T: TimestampExchanger>(
    timestamp_exchanger: &T,
) -> Result<ClockOffset, CristianError> {
    sample_offset(timestamp_exchanger)
}

/// Samples the clock offset once using Cristian's Algorithm.
///
/// This is a helper function that performs a single timestamp exchange and
/// calculates the clock offset based on Cristian's algorithm.
///
/// # Arguments
///
/// * `timestamp_exchanger`: An implementation of `TimestampExchanger` trait.
///
/// # Returns
///
/// * `Ok(ClockOffset)`: The calculated clock offset in nanoseconds for this sample.
/// * `Err(CristianError)`: If the sampling fails due to communication errors.
fn sample_offset<T: TimestampExchanger>(
    timestamp_exchanger: &T,
) -> Result<ClockOffset, CristianError> {
    let t1 = Instant::now();
    let server_timestamp_result = timestamp_exchanger.exchange_timestamp();
    let t2 = Instant::now();

    match server_timestamp_result {
        Ok(ts) => {
            let rtt = t2.duration_since(t1).as_nanos() as Timestamp;
            // Cristian's Algorithm offset calculation:
            // Offset = Server Timestamp - (t1 + RTT/2)
            let offset = ts - (t1.elapsed().as_nanos() as Timestamp + rtt / 2);
            Ok(offset)
        }
        Err(e) => Err(e), // Propagate the error from timestamp exchange
    }
}

/// Calculates the median of a slice of clock offsets.
///
/// This function sorts the offsets and returns the median value.
/// If the number of offsets is even, it returns the median of the two middle values
/// (lower median is chosen in integer division for simplicity).
///
/// # Arguments
///
/// * `offsets`: A slice of `ClockOffset` values.
///
/// # Returns
///
/// The median clock offset.
fn calculate_median_offset(offsets: &[ClockOffset]) -> Result<ClockOffset, CristianError> {
    if offsets.is_empty() {
        return Err(CristianError::NoValidTimestamps);
    }

    let mut sorted_offsets = offsets.to_vec();
    sorted_offsets.sort_unstable(); // For performance, order doesn't matter

    let mid = (sorted_offsets.len() - 1) / 2;
    Ok(sorted_offsets[mid]) // For even length, lower median is returned. Can adjust if needed.
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    use super::*;

    // Mock TimestampExchanger for testing purposes
    struct MockTestExchanger {
        server_time_offset: ClockOffset, // Simulate server clock ahead/behind
        network_delay_ms: u64,
        fail_exchange_nth_call: Cell<Option<u32>>, // Simulate exchange failure on nth call
        exchange_call_count: Cell<u32>,
    }

    impl MockTestExchanger {
        fn new(server_time_offset: ClockOffset, network_delay_ms: u64) -> Self {
            MockTestExchanger {
                server_time_offset,
                network_delay_ms,
                fail_exchange_nth_call: Cell::new(None),
                exchange_call_count: Cell::new(0),
            }
        }

        fn with_failure_on_nth_call(self, nth_call: u32) -> Self {
            self.fail_exchange_nth_call.set(Some(nth_call));
            self
        }
    }

    impl TimestampExchanger for MockTestExchanger {
        fn exchange_timestamp(&self) -> Result<Timestamp, CristianError> {
            let call_count = self.exchange_call_count.get();
            self.exchange_call_count.set(call_count + 1);

            if let Some(fail_nth) = self.fail_exchange_nth_call.get() {
                if call_count + 1 == fail_nth {
                    return Err(CristianError::ExchangeError(
                        "Simulated exchange failure".to_string(),
                    ));
                }
            }

            std::thread::sleep(Duration::from_millis(self.network_delay_ms)); // Simulate network delay
            let current_time = Instant::now().elapsed().as_nanos() as Timestamp;
            Ok(current_time + self.server_time_offset) // Simulate server time
        }
    }

    #[test]
    fn test_initial_sync_no_offset() {
        let config = CristianConfig { num_samples: 5 };
        let exchanger = MockTestExchanger::new(0, 10); // Server time same, 10ms delay
        let result = initial_sync(config, &exchanger);
        assert!(result.is_ok());
        let offset = result.unwrap();
        println!("Offset: {}", offset);
        assert!(offset.abs() < 5_000_000); // Offset within acceptable range (5ms tolerance in ns)
    }

    #[test]
    fn test_initial_sync_positive_offset() {
        let config = CristianConfig { num_samples: 5 };
        let exchanger = MockTestExchanger::new(10_000_000, 10); // Server 10ms ahead, 10ms delay
        let result = initial_sync(config, &exchanger);
        assert!(result.is_ok());
        let offset = result.unwrap();
        println!("Offset: {}", offset);
        assert!((offset - 10_000_000).abs() < 5_000_000); // Offset close to 10ms
    }

    #[test]
    fn test_initial_sync_negative_offset() {
        let config = CristianConfig { num_samples: 5 };
        let exchanger = MockTestExchanger::new(-10_000_000, 10); // Server 10ms behind, 10ms delay
        let result = initial_sync(config, &exchanger);
        assert!(result.is_ok());
        let offset = result.unwrap();
        println!("Offset: {}", offset);
        assert!((offset + 10_000_000).abs() < 5_000_000); // Offset close to -10ms
    }

    #[test]
    fn test_initial_sync_with_failure() {
        let config = CristianConfig { num_samples: 5 };
        let exchanger = MockTestExchanger::new(10_000_000, 10).with_failure_on_nth_call(3); // Fail on 3rd call
        let result = initial_sync(config, &exchanger);
        assert!(result.is_ok()); // Still ok if some samples are valid
        let offset = result.unwrap();
        println!("Offset with failure: {}", offset);
        assert!((offset - 10_000_000).abs() < 5_000_000); // Should still get reasonable offset
    }

    #[test]
    fn test_initial_sync_all_failures() {
        let config = CristianConfig { num_samples: 3 };
        let exchanger = MockTestExchanger::new(0, 10).with_failure_on_nth_call(1); // Fail on every call
        let result = initial_sync(config, &exchanger);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CristianError::NoValidTimestamps
        ));
    }

    #[test]
    fn test_check_offset() {
        let exchanger = MockTestExchanger::new(5_000_000, 20); // Server 5ms ahead, 20ms delay
        let result = check_offset(&exchanger);
        assert!(result.is_ok());
        let offset = result.unwrap();
        println!("Check Offset: {}", offset);
        assert!((offset - 5_000_000).abs() < 5_000_000); // Offset close to 5ms
    }

    #[test]
    fn test_check_offset_failure() {
        let exchanger = MockTestExchanger::new(0, 0).with_failure_on_nth_call(1); // Fail immediately
        let result = check_offset(&exchanger);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CristianError::ExchangeError(_)
        ));
    }

    #[test]
    fn test_calculate_median_offset_odd() {
        let offsets = vec![10, 20, 30, 5, 15];
        let median = calculate_median_offset(&offsets).unwrap();
        assert_eq!(median, 15);
    }

    #[test]
    fn test_calculate_median_offset_even() {
        let offsets = vec![10, 20, 30, 5];
        let median = calculate_median_offset(&offsets).unwrap();
        // Lower median is chosen, which is 10 in sorted [5, 10, 20, 30]
        assert_eq!(median, 10);
    }

    #[test]
    fn test_calculate_median_offset_empty() {
        let offsets = vec![];
        let result = calculate_median_offset(&offsets);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CristianError::NoValidTimestamps
        ));
    }
}
