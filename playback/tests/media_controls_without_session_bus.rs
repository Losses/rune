//! OS media controls on a Linux machine without a D-Bus session bus (no desktop session, a
//! container, a minimal window manager session).

#![cfg(target_os = "linux")]

use std::env;

use playback::controller::MediaControlManager;

#[test]
fn initialize_without_a_session_bus_is_an_error() {
    // SAFETY: this is the only test of this binary, so no other thread reads or writes the
    // environment while it is changed.
    unsafe {
        env::set_var(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/rune-test-session-bus",
        );
    }

    let mut manager =
        MediaControlManager::with_name("rune_test_no_session_bus").expect("create the manager");
    let error = manager
        .initialize()
        .expect_err("media controls cannot start without a session bus");
    // The caller falls back on this error; the service thread that used to panic instead is
    // never started.
    assert!(
        manager.controls.detach().is_ok(),
        "no media control thread panicked ({error:#})"
    );
}
