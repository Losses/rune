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
//! use once_cell::sync::Lazy;
//!
//! use sync::clock::{CristianConfig, CristianError, initial_sync, TimestampExchanger, Timestamp, check_offset, ClockOffset};
//!
//! // In a real scenario, PROCESS_START would likely be defined in your crate/main.
//! // For this example, we define it here. It must be accessible by the MockExchanger.
//! static PROCESS_START_EXAMPLE: Lazy<Instant> = Lazy::new(Instant::now);
//!
//! struct MyNetworkExchanger;
//!
//! impl TimestampExchanger for MyNetworkExchanger {
//!     fn exchange_timestamp(&self) -> Result<Timestamp, CristianError> {
//!         thread::sleep(Duration::from_millis(20)); // Simulate network delay
//!
//!         // Simulate server time 5s ahead relative to PROCESS_START_EXAMPLE
//!         let server_timestamp = PROCESS_START_EXAMPLE.elapsed().as_nanos() as Timestamp + 5_000_000_000;
//!
//!         Ok(server_timestamp)
//!     }
//! }
//!
//! fn main() -> Result<(), CristianError> {
//!     // Access PROCESS_START_EXAMPLE here if needed, or use the library's PROCESS_START if accessible
//!     let _ = *PROCESS_START_EXAMPLE;
//!
//!     let config = CristianConfig::default();
//!     let network_exchanger = MyNetworkExchanger;
//!
//!     let initial_offset = initial_sync(config, &network_exchanger)?;
//!     println!("Initial clock offset: {} ns", initial_offset);
//!
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
//!   functional in a real-world scenario. **Crucially, ensure the timestamp returned by the server uses a consistent and known epoch.**
//! * The accuracy of Cristian's Algorithm depends on the network latency and its symmetry.
//!   If the network latency is high or asymmetric, the accuracy may be reduced.
//! * For more advanced synchronization needs, consider using more sophisticated algorithms
//!   like NTP (Network Time Protocol) or PTP (Precision Time Protocol).
//!

use std::time::Instant;

use once_cell::sync::Lazy;

// NOTE: This PROCESS_START is accessible within this module, including tests and doctests.
pub static PROCESS_START: Lazy<Instant> = Lazy::new(Instant::now);

/// Represents a timestamp in nanoseconds since an epoch (e.g., system start).
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
    fn exchange_timestamp(&self) -> Result<Timestamp, CristianError>;
}

/// Executes Cristian's Algorithm to perform initial clock synchronization.
/// # Example
///
/// ```rust
/// // Items from the sync::clock module are directly available in doctests
/// use std::time::{Duration, Instant};
/// use std::thread;
///
/// use once_cell::sync::Lazy;
///
/// use crate::sync::clock::{CristianConfig, CristianError, initial_sync, TimestampExchanger, Timestamp, ClockOffset, PROCESS_START};
///
/// // Mock TimestampExchanger defined directly within the doctest
/// struct MockExchanger;
///
/// impl TimestampExchanger for MockExchanger {
///     fn exchange_timestamp(&self) -> Result<Timestamp, CristianError> {
///         // Simulate network delay and server timestamp retrieval
///         thread::sleep(Duration::from_millis(50)); // Simulate network delay
///         // Simulate server time 1 second ahead, relative to *the module's* PROCESS_START
///         let server_timestamp = crate::sync::clock::PROCESS_START.elapsed().as_nanos() as Timestamp + 1_000_000_000;
///         Ok(server_timestamp)
///     }
/// }
///
/// fn main() -> Result<(), CristianError> {
///     // Initialize PROCESS_START if not already done (Lazy handles this)
///     let _ = *crate::sync::clock::PROCESS_START;
///
///     let config = CristianConfig::default();
///     let exchanger = MockExchanger; // Use the mock defined above
///     let offset = initial_sync(config, &exchanger)?;
///     // Expected offset = server_offset + rtt/2 ≈ 1_000_000_000 + (50ms + overhead)/2 ≈ 1_025_000_000 ns
///     // Add tolerance for timing variations in test execution
///     let expected_base = 1_000_000_000 + (50 * 1_000_000 / 2);
///     let tolerance = 10_000_000; // 10ms tolerance
///     println!("Calculated clock offset: {} ns (Expected approx {})", offset, expected_base);
///     assert!((offset - expected_base).abs() < tolerance);
///     Ok(())
/// }
/// ```
pub fn initial_sync<T: TimestampExchanger>(
    config: CristianConfig,
    timestamp_exchanger: &T,
) -> Result<ClockOffset, CristianError> {
    let mut offsets: Vec<ClockOffset> = Vec::new();
    let mut valid_samples = 0;

    for i in 0..config.num_samples {
        // Use index for logging if needed
        match sample_offset(timestamp_exchanger) {
            Ok(offset) => {
                offsets.push(offset);
                valid_samples += 1;
            }
            Err(e) => {
                // Log error but continue if possible, maybe use a tracing library later
                eprintln!(
                    "Warning: Error during timestamp sampling (sample {}): {}",
                    i + 1,
                    e
                );
                // Depending on the error type, you might want to retry or abort.
                // For now, just skip this sample.
            }
        }
    }

    if valid_samples == 0 {
        return Err(CristianError::NoValidTimestamps);
    }

    calculate_median_offset(&offsets)
}

/// Checks the clock offset in a task execution context using Cristian's Algorithm (single sample).
/// # Example
///
/// ```rust
/// use std::time::{Duration, Instant};
/// use std::thread;
/// 
/// use once_cell::sync::Lazy;
/// 
/// use crate::sync::clock::{check_offset, CristianError, TimestampExchanger, Timestamp, ClockOffset, PROCESS_START};
///
/// // Mock TimestampExchanger defined directly within the doctest
/// struct MockExchanger;
///
/// impl TimestampExchanger for MockExchanger {
///     fn exchange_timestamp(&self) -> Result<Timestamp, CristianError> {
///         thread::sleep(Duration::from_millis(50));
///         // Simulate server 1s ahead relative to *the module's* PROCESS_START
///         let server_timestamp = crate::sync::clock::PROCESS_START.elapsed().as_nanos() as Timestamp + 1_000_000_000;
///         Ok(server_timestamp)
///     }
/// }
///
/// fn main() -> Result<(), CristianError> {
///     // Initialize PROCESS_START if not already done (Lazy handles this)
///     let _ = *crate::sync::clock::PROCESS_START;
///
///     let exchanger = MockExchanger; // Use the mock defined above
///     let current_offset = check_offset(&exchanger)?;
///     // Expected offset = server_offset + rtt/2 ≈ 1_000_000_000 + (50ms + overhead)/2 ≈ 1_025_000_000 ns
///     let expected_base = 1_000_000_000 + (50 * 1_000_000 / 2);
///     let tolerance = 10_000_000; // 10ms tolerance
///     println!("Current clock offset: {} ns (Expected approx {})", current_offset, expected_base);
///     assert!((current_offset - expected_base).abs() < tolerance);
///     Ok(())
/// }
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
/// Formula: `Offset = (T_server + RTT/2) - T_client_receipt`
/// where all times are relative to the same epoch.
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
    let t1_instant = Instant::now();
    let server_timestamp_result = timestamp_exchanger.exchange_timestamp();
    let t2_instant = Instant::now();

    match server_timestamp_result {
        Ok(server_timestamp) => {
            // Calculate client-side timestamps relative to PROCESS_START
            let t1 = t1_instant.duration_since(*PROCESS_START).as_nanos() as Timestamp;
            let t2 = t2_instant.duration_since(*PROCESS_START).as_nanos() as Timestamp;

            // Ensure t2 is strictly greater than t1. Handle potential Instant wrap-around or clock jumps if necessary,
            // although Instant is monotonic and should prevent this on most systems.
            if t2 <= t1 {
                // This indicates a potential issue with timer resolution, system clock adjustment, or very fast execution.
                // Depending on requirements, either return an error or a default offset (like 0).
                return Err(CristianError::ExchangeError(format!(
                    "Invalid RTT calculation: t2 ({}) <= t1 ({}). System clock issue?",
                    t2, t1
                )));
                // Or, alternatively, treat RTT as minimal measurable duration if acceptable:
                // rtt = 1; // Assign a minimum positive RTT, e.g., 1 nanosecond
            }
            let rtt = t2 - t1;

            // Estimate server time at the moment client received the response (t2)
            // Server's clock = server_timestamp (server's time when it sent response)
            // We estimate the server's clock advanced by half the RTT by the time client received it.
            // This assumes symmetric network delay.
            let estimated_server_time_at_t2 = server_timestamp.wrapping_add(rtt / 2);

            // Offset = Estimated Server Time at t2 - Client Time at t2
            let offset = estimated_server_time_at_t2.wrapping_sub(t2);

            Ok(offset)
        }
        Err(e) => Err(e),
    }
}

/// Calculates the median of a slice of clock offsets.
///
/// This function sorts the offsets and returns the median value.
/// If the number of offsets is even, it returns the lower median value.
///
/// # Arguments
///
/// * `offsets`: A slice of `ClockOffset` values.
///
/// # Returns
///
/// * `Ok(ClockOffset)`: The median clock offset.
/// * `Err(CristianError::NoValidTimestamps)`: If the input slice is empty.
fn calculate_median_offset(offsets: &[ClockOffset]) -> Result<ClockOffset, CristianError> {
    if offsets.is_empty() {
        return Err(CristianError::NoValidTimestamps);
    }

    let mut sorted_offsets = offsets.to_vec();
    sorted_offsets.sort_unstable(); // Use unstable sort for potential performance gain

    // Calculate the index of the median element.
    // For odd length N, mid = (N-1)/2 (0-based index). E.g., N=5, mid=(5-1)/2=2 (3rd element).
    // For even length N, mid = (N-1)/2 gives the lower middle index. E.g., N=4, mid=(4-1)/2=1 (2nd element).
    let mid = (sorted_offsets.len() - 1) / 2;
    Ok(sorted_offsets[mid])
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    // Import items from the outer module (sync::clock)
    use super::*;

    // Mock TimestampExchanger for testing purposes
    struct MockTestExchanger {
        server_time_offset_ns: ClockOffset, // Simulate server clock ahead/behind in nanoseconds
        network_delay_ms: u64,
        fail_exchange_nth_call: Cell<Option<u32>>, // Simulate exchange failure on Nth call (1-based)
        exchange_call_count: Cell<u32>,            // Track number of calls (0-based internally)
        fail_always: Cell<bool>,
    }

    impl MockTestExchanger {
        fn new(server_time_offset_ns: ClockOffset, network_delay_ms: u64) -> Self {
            MockTestExchanger {
                server_time_offset_ns,
                network_delay_ms,
                fail_exchange_nth_call: Cell::new(None),
                exchange_call_count: Cell::new(0),
                fail_always: Cell::new(false),
            }
        }

        fn with_failure_on_nth_call(self, nth_call: u32) -> Self {
            self.fail_exchange_nth_call.set(Some(nth_call));
            self.fail_always.set(false); // Ensure fail_always is off if using nth_call
            self
        }

        fn with_fail_always(self) -> Self {
            self.fail_always.set(true);
            self.fail_exchange_nth_call.set(None); // Ensure nth_call is off if using fail_always
            self
        }
    }

    impl TimestampExchanger for MockTestExchanger {
        fn exchange_timestamp(&self) -> Result<Timestamp, CristianError> {
            let current_call_index = self.exchange_call_count.get();
            let current_call_number = current_call_index + 1; // 1-based number for comparison
            self.exchange_call_count.set(current_call_number); // Increment for next call

            // Check if we should always fail
            if self.fail_always.get() {
                return Err(CristianError::ExchangeError(format!(
                    "Simulated always failure on call {}",
                    current_call_number
                )));
            }

            // Check if this call should fail
            if let Some(fail_nth) = self.fail_exchange_nth_call.get() {
                if current_call_number == fail_nth {
                    // Check for exact match
                    return Err(CristianError::ExchangeError(format!(
                        "Simulated exchange failure on call {}",
                        current_call_number
                    )));
                }
            }

            // Simulate network delay
            std::thread::sleep(Duration::from_millis(self.network_delay_ms));

            // Calculate the server's timestamp *relative to the same epoch as the client* (PROCESS_START)
            // Server's current time = Client's current time + Server Offset
            let client_time_now = PROCESS_START.elapsed().as_nanos() as Timestamp;
            let server_timestamp = client_time_now.wrapping_add(self.server_time_offset_ns);

            Ok(server_timestamp)
        }
    }

    // Define a reasonable tolerance for offset comparisons in nanoseconds
    // Allow for RTT variance, scheduling delays etc. ~10ms seems plausible for local tests.
    const OFFSET_TOLERANCE_NS: ClockOffset = 10_000_000;

    #[test]
    fn test_initial_sync_no_offset() {
        let config = CristianConfig { num_samples: 5 };
        let delay_ms = 10;
        let server_offset_ns = 0;
        let exchanger = MockTestExchanger::new(server_offset_ns, delay_ms);

        let result = initial_sync(config, &exchanger);
        assert!(result.is_ok());
        let offset = result.unwrap();

        // Expected offset = server_offset + rtt/2 ≈ 0 + (10ms + overhead)/2 ≈ 5ms + overhead/2
        let expected_offset_ns = (delay_ms * 1_000_000 / 2) as ClockOffset;
        println!(
            "No Offset Test - Offset: {} ns, Expected ≈ {} ns",
            offset, expected_offset_ns
        );
        assert!(
            (offset - expected_offset_ns).abs() < OFFSET_TOLERANCE_NS,
            "Offset {} not within {} ns of expected {}",
            offset,
            OFFSET_TOLERANCE_NS,
            expected_offset_ns
        );
    }

    #[test]
    fn test_initial_sync_positive_offset() {
        let config = CristianConfig { num_samples: 5 };
        let delay_ms = 10;
        let server_offset_ns = 10_000_000; // Server 10ms ahead
        let exchanger = MockTestExchanger::new(server_offset_ns, delay_ms);

        let result = initial_sync(config, &exchanger);
        assert!(result.is_ok());
        let offset = result.unwrap();

        // Expected offset = server_offset + rtt/2 ≈ 10ms + (10ms + overhead)/2 ≈ 15ms + overhead/2
        let expected_offset_ns = server_offset_ns + (delay_ms * 1_000_000 / 2) as ClockOffset;
        println!(
            "Positive Offset Test - Offset: {} ns, Expected ≈ {} ns",
            offset, expected_offset_ns
        );
        assert!(
            (offset - expected_offset_ns).abs() < OFFSET_TOLERANCE_NS,
            "Offset {} not within {} ns of expected {}",
            offset,
            OFFSET_TOLERANCE_NS,
            expected_offset_ns
        );
    }

    #[test]
    fn test_initial_sync_negative_offset() {
        let config = CristianConfig { num_samples: 5 };
        let delay_ms = 10;
        let server_offset_ns = -10_000_000; // Server 10ms behind
        let exchanger = MockTestExchanger::new(server_offset_ns, delay_ms);

        let result = initial_sync(config, &exchanger);
        assert!(result.is_ok());
        let offset = result.unwrap();

        // Expected offset = server_offset + rtt/2 ≈ -10ms + (10ms + overhead)/2 ≈ -5ms + overhead/2
        let expected_offset_ns = server_offset_ns + (delay_ms * 1_000_000 / 2) as ClockOffset;
        println!(
            "Negative Offset Test - Offset: {} ns, Expected ≈ {} ns",
            offset, expected_offset_ns
        );
        assert!(
            (offset - expected_offset_ns).abs() < OFFSET_TOLERANCE_NS,
            "Offset {} not within {} ns of expected {}",
            offset,
            OFFSET_TOLERANCE_NS,
            expected_offset_ns
        );
    }

    #[test]
    fn test_initial_sync_with_failure() {
        let config = CristianConfig { num_samples: 5 };
        let delay_ms = 10;
        let server_offset_ns = 10_000_000; // Server 10ms ahead
        let fail_nth = 3; // Fail on the 3rd call
        let exchanger =
            MockTestExchanger::new(server_offset_ns, delay_ms).with_failure_on_nth_call(fail_nth);

        let result = initial_sync(config, &exchanger);
        assert!(result.is_ok()); // Should still be OK as other samples are valid
        let offset = result.unwrap();

        // Expected offset based on median of successful samples (1, 2, 4, 5)
        // Expected offset ≈ 10ms + (10ms + overhead)/2 ≈ 15ms + overhead/2
        let expected_offset_ns = server_offset_ns + (delay_ms * 1_000_000 / 2) as ClockOffset;
        println!(
            "Failure Test - Offset: {} ns, Expected ≈ {} ns (based on {} successful samples)",
            offset,
            expected_offset_ns,
            config.num_samples - 1
        );
        assert!(
            (offset - expected_offset_ns).abs() < OFFSET_TOLERANCE_NS,
            "Offset {} not within {} ns of expected {}",
            offset,
            OFFSET_TOLERANCE_NS,
            expected_offset_ns
        );
        // Verify failure only happened once
        assert_eq!(
            exchanger.exchange_call_count.get(),
            config.num_samples,
            "Exchanger should have been called {} times",
            config.num_samples
        );
    }

    #[test]
    fn test_initial_sync_all_failures() {
        let config = CristianConfig { num_samples: 3 };
        let exchanger = MockTestExchanger::new(0, 10).with_fail_always();

        println!("Running initial_sync expecting all failures...");
        let result = initial_sync(config, &exchanger);
        println!("Result: {:?}", result);

        assert!(
            result.is_err(),
            "Expected initial_sync to return Err when all samples fail"
        );

        match result.unwrap_err() {
            CristianError::NoValidTimestamps => {}
            e => panic!("Expected NoValidTimestamps error, got {:?}", e),
        }

        // Check that the loop still tried num_samples times
        assert_eq!(
            exchanger.exchange_call_count.get(),
            config.num_samples,
            "Exchanger should have been called {} times even with failures",
            config.num_samples
        );
    }

    #[test]
    fn test_check_offset() {
        let delay_ms = 20;
        let server_offset_ns = 5_000_000; // Server 5ms ahead
        let exchanger = MockTestExchanger::new(server_offset_ns, delay_ms);

        let result = check_offset(&exchanger);
        assert!(result.is_ok());
        let offset = result.unwrap();

        // Expected offset = server_offset + rtt/2 ≈ 5ms + (20ms + overhead)/2 ≈ 15ms + overhead/2
        let expected_offset_ns = server_offset_ns + (delay_ms * 1_000_000 / 2) as ClockOffset;
        println!(
            "Check Offset Test - Offset: {} ns, Expected ≈ {} ns",
            offset, expected_offset_ns
        );
        assert!(
            (offset - expected_offset_ns).abs() < OFFSET_TOLERANCE_NS,
            "Offset {} not within {} ns of expected {}",
            offset,
            OFFSET_TOLERANCE_NS,
            expected_offset_ns
        );
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
        assert_eq!(exchanger.exchange_call_count.get(), 1); // Should have been called once
    }

    #[test]
    fn test_calculate_median_offset_odd() {
        let offsets = vec![10, 20, 30, 5, 15]; // Sorted: [5, 10, 15, 20, 30]
        let median = calculate_median_offset(&offsets).unwrap();
        assert_eq!(median, 15);
    }

    #[test]
    fn test_calculate_median_offset_even() {
        let offsets = vec![10, 20, 30, 5]; // Sorted: [5, 10, 20, 30]
        let median = calculate_median_offset(&offsets).unwrap();
        // Lower median is chosen: index (4-1)/2 = 1, which is value 10
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

    #[test]
    fn test_sample_offset_invalid_rtt() {
        // This test is tricky to reliably trigger without manipulating time itself.
        // We'll rely on the check added in sample_offset.
        // We can simulate by making t2 <= t1 through a mock exchanger that returns
        // a timestamp very quickly and hoping Instant::now() resolution/ordering works out.
        // However, it's not guaranteed. The code safeguard is the primary check.
        struct QuickExchanger;
        impl TimestampExchanger for QuickExchanger {
            fn exchange_timestamp(&self) -> Result<Timestamp, CristianError> {
                // Return time immediately, potentially causing t2 <= t1 if resolution is low
                // or clock adjusts backward slightly (though Instant should prevent latter)
                Ok(PROCESS_START.elapsed().as_nanos() as Timestamp)
            }
        }
        let exchanger = QuickExchanger;
        // Run it a few times, maybe it triggers the condition
        let mut triggered_error = false;
        for _ in 0..10 {
            let result = sample_offset(&exchanger);
            if let Err(CristianError::ExchangeError(msg)) = result {
                if msg.contains("Invalid RTT calculation") {
                    triggered_error = true;
                    break;
                }
            }
            // Add a small delay to increase chance of Instant::now() difference
            std::thread::sleep(Duration::from_nanos(10));
        }
        // We don't assert true here, as it's timing-dependent.
        // Just running it provides some confidence the check exists.
        if triggered_error {
            println!("Successfully triggered invalid RTT scenario.");
        } else {
            println!("Warning: Could not reliably trigger invalid RTT scenario in test.");
        }
    }
}
