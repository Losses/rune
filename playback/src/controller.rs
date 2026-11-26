#[cfg(target_os = "windows")]
use std::ffi::c_void;
use std::io::Write;
use std::time::Duration;
use std::{env, fs::File};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use anyhow::{Error, Result, bail};
use log::{debug, info};
use once_cell::sync::OnceCell;
use tokio::sync::Mutex;

#[cfg(target_os = "android")]
use crate::android_media_session::MediaControls;
#[cfg(target_os = "android")]
use crate::dummy_souvlaki::{MediaControlEvent, PlatformConfig, SeekDirection};

#[cfg(target_os = "ios")]
use crate::dummy_souvlaki::{MediaControlEvent, PlatformConfig, SeekDirection};
#[cfg(target_os = "ios")]
use crate::ios_now_playing::MediaControls;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig, SeekDirection};

use simple_channel::{SimpleChannel, SimpleReceiver, SimpleSender};

use crate::player::{Playable, PlaybackState};

// Use include_bytes! to embed the image into the binary
const IMAGE_DATA: &[u8] = include_bytes!("default_cover_art.png");

// Use OnceCell to store the temporary file path
static IMAGE_PATH: OnceCell<PathBuf> = OnceCell::new();

pub fn get_default_cover_art_path() -> &'static Path {
    IMAGE_PATH.get_or_init(|| {
        // Get the path to the temporary directory
        let temp_dir = env::temp_dir();
        let image_path = temp_dir.join("default_cover_art.png");

        // Write the image data to the temporary file
        let mut file = File::create(&image_path).expect("Failed to create file");
        file.write_all(IMAGE_DATA).expect("Failed to write data");

        image_path
    })
}

pub struct MediaControlManager {
    pub controls: MediaControls,
    event_sender: SimpleSender<MediaControlEvent>,
    // Dropped after `controls` so the hidden HWND stays valid until SMTC is released.
    #[cfg(target_os = "windows")]
    _dummy_window: windows::DummyWindow,
}

impl MediaControlManager {
    pub fn new() -> Result<Self> {
        Self::with_name("rune_player")
    }

    /// Use a distinct OS media-control identity for each node session.
    pub fn with_name(dbus_name: &str) -> Result<Self> {
        #[cfg(not(any(target_os = "windows")))]
        let hwnd = None;

        #[cfg(target_os = "windows")]
        let (hwnd, dummy_window) = {
            let dummy_window = windows::DummyWindow::new()?;
            let handle = dummy_window.handle.0 as *mut c_void;
            (Some(handle), dummy_window)
        };

        let config = PlatformConfig {
            dbus_name,
            display_name: "Rune",
            hwnd,
        };

        let controls = match MediaControls::new(config) {
            Ok(x) => x,
            Err(e) => bail!(Error::msg(format!("{e:?}"))),
        };

        let (event_sender, _) = SimpleChannel::channel(32);

        Ok(Self {
            controls,
            event_sender,
            #[cfg(target_os = "windows")]
            _dummy_window: dummy_window,
        })
    }

    pub fn initialize(&mut self) -> Result<()> {
        info!("Initializing media controls");

        let event_sender = self.event_sender.clone();
        let request = self.controls.attach(move |event: MediaControlEvent| {
            let event_sender = event_sender.clone();
            thread::spawn(move || {
                event_sender.send(event);
            });
        });

        match request {
            Ok(x) => x,
            Err(e) => bail!(Error::msg(format!("{e:?}"))),
        };

        Ok(())
    }

    pub fn subscribe_controller_events(&self) -> SimpleReceiver<MediaControlEvent> {
        self.event_sender.subscribe()
    }

    /// A platform host plays the current track and shows Now Playing for it itself. On iOS,
    /// where such a host does that in the app's own place, Rune's Now Playing gives way until
    /// [`Self::reclaim_commands`]; elsewhere nothing changes.
    pub fn yield_commands(&mut self) -> Result<()> {
        #[cfg(target_os = "ios")]
        if let Err(e) = self.controls.yield_commands() {
            bail!(Error::msg(format!("{e:?}")));
        }
        Ok(())
    }

    /// The host let go of Now Playing, and Rune's own comes back.
    pub fn reclaim_commands(&mut self) -> Result<()> {
        #[cfg(target_os = "ios")]
        if let Err(e) = self.controls.reclaim_commands() {
            bail!(Error::msg(format!("{e:?}")));
        }
        Ok(())
    }

    /// Whether a platform host plays the current track, whose player looks after the audio
    /// itself. On Android, where the media controls hold the audio focus for Rune's own player,
    /// they then give it up and ask for none, so that the host's player keeps the focus it asks
    /// for; elsewhere nothing changes.
    pub fn set_hosted_audio(&mut self, hosted: bool) -> Result<()> {
        #[cfg(target_os = "android")]
        if let Err(e) = self.controls.set_hosted_audio(hosted) {
            bail!(Error::msg(format!("{e:?}")));
        }
        #[cfg(not(target_os = "android"))]
        let _ = hosted;
        Ok(())
    }
}

pub async fn handle_media_control_event(
    player: &Arc<Mutex<dyn Playable>>,
    event: MediaControlEvent,
) -> Result<()> {
    debug!("Received media control event: {event:?}");

    match event {
        MediaControlEvent::Play => player.lock().await.play(),
        MediaControlEvent::Pause => player.lock().await.pause(),
        MediaControlEvent::Toggle => {
            if player.lock().await.get_status().state == PlaybackState::Playing {
                player.lock().await.pause();
            } else {
                player.lock().await.play();
            }
        }
        MediaControlEvent::Next => player.lock().await.next(),
        MediaControlEvent::Previous => player.lock().await.previous(),
        MediaControlEvent::Stop => player.lock().await.stop(),
        MediaControlEvent::Seek(_) | MediaControlEvent::SetPosition(_) => {
            let current_position = player.lock().await.get_status().position;
            if let Some(target) = seek_target(&event, current_position) {
                player.lock().await.seek(target);
            }
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        _ => debug!("Unhandled media control event: {event:?}"),
    }

    Ok(())
}

/// Where a seek or position event asks the player to go, in seconds, which is what
/// [`Playable::seek`] takes. A seek moves 10 s from `current_position`, and no further back than
/// the start.
fn seek_target(event: &MediaControlEvent, current_position: Duration) -> Option<f64> {
    match event {
        MediaControlEvent::Seek(direction) => {
            let step = match direction {
                SeekDirection::Forward => 10.0,
                SeekDirection::Backward => -10.0,
            };
            Some((current_position.as_secs_f64() + step).max(0.0))
        }
        MediaControlEvent::SetPosition(position) => Some(position.0.as_secs_f64()),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::io::Error;
    #[cfg(windows)]
    use std::mem;
    use std::sync::mpsc;
    use std::thread::{self, JoinHandle};

    use anyhow::{Context, Result, bail};

    use windows::Win32::Foundation::{
        ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IsWindow,
        MSG, PostMessageW, PostQuitMessage, PostThreadMessageW, RegisterClassExW, TranslateMessage,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_QUIT, WNDCLASSEXW,
    };
    use windows::core::w;

    pub struct DummyWindow {
        pub handle: HWND,
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
    }

    impl DummyWindow {
        pub fn new() -> Result<DummyWindow> {
            let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(isize, u32)>>(1);

            let thread = thread::Builder::new()
                .name("rune-media-control-window".to_owned())
                .spawn(move || {
                    let handle = match Self::create_window() {
                        Ok(handle) => handle,
                        Err(err) => {
                            let _ = ready_tx.send(Err(err));
                            return;
                        }
                    };

                    let thread_id = unsafe { GetCurrentThreadId() };
                    if ready_tx.send(Ok((handle.0, thread_id))).is_err() {
                        unsafe {
                            DestroyWindow(handle);
                        }
                        return;
                    }

                    unsafe {
                        let mut msg = MSG::default();
                        while GetMessageW(&mut msg, HWND(0), 0, 0).0 > 0 {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                        if IsWindow(handle).as_bool() {
                            DestroyWindow(handle);
                        }
                    }
                })
                .context("Failed to spawn media control window thread")?;

            let ready = match ready_rx.recv() {
                Ok(result) => result,
                Err(_) => {
                    let _ = thread.join();
                    bail!("Media control window thread exited before creating window");
                }
            };

            match ready {
                Ok((raw_handle, thread_id)) => Ok(DummyWindow {
                    handle: HWND(raw_handle),
                    thread_id,
                    thread: Some(thread),
                }),
                Err(err) => {
                    let _ = thread.join();
                    Err(err)
                }
            }
        }

        fn create_window() -> Result<HWND> {
            let class_name = w!("SimpleTray");

            unsafe {
                let instance =
                    GetModuleHandleW(None).with_context(|| "Unable to get module handle")?;

                let wnd_class = WNDCLASSEXW {
                    cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
                    hInstance: instance,
                    lpszClassName: class_name,
                    lpfnWndProc: Some(Self::wnd_proc),
                    ..Default::default()
                };

                if RegisterClassExW(&wnd_class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS
                {
                    bail!("Registering class failed: {}", Error::last_os_error());
                }

                let handle = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class_name,
                    w!(""),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    instance,
                    None,
                );

                if handle.0 == 0 {
                    bail!(
                        "Message only window creation failed: {}",
                        Error::last_os_error()
                    )
                } else {
                    Ok(handle)
                }
            }
        }

        extern "system" fn wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            unsafe {
                if msg == WM_DESTROY {
                    PostQuitMessage(0);
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
    }

    impl Drop for DummyWindow {
        fn drop(&mut self) {
            unsafe {
                if !PostMessageW(self.handle, WM_CLOSE, WPARAM(0), LPARAM(0)).as_bool() {
                    PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            }
            if let Some(thread) = self.thread.take()
                && thread::current().id() != thread.thread().id()
            {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MediaPosition;

    #[test]
    fn a_position_from_the_lock_screen_is_given_in_seconds() {
        let event = MediaControlEvent::SetPosition(MediaPosition(Duration::from_millis(63_959)));
        assert_eq!(seek_target(&event, Duration::from_secs(5)), Some(63.959));
    }

    #[test]
    fn a_seek_moves_ten_seconds_from_where_the_track_is() {
        let forward = MediaControlEvent::Seek(SeekDirection::Forward);
        let backward = MediaControlEvent::Seek(SeekDirection::Backward);
        assert_eq!(seek_target(&forward, Duration::from_secs(30)), Some(40.0));
        assert_eq!(seek_target(&backward, Duration::from_secs(30)), Some(20.0));
    }

    #[test]
    fn a_seek_back_near_the_start_goes_to_the_start() {
        let backward = MediaControlEvent::Seek(SeekDirection::Backward);
        assert_eq!(seek_target(&backward, Duration::from_secs(4)), Some(0.0));
    }

    #[test]
    fn other_events_do_not_seek() {
        assert_eq!(seek_target(&MediaControlEvent::Play, Duration::ZERO), None);
    }

    #[cfg(target_os = "windows")]
    mod windows_tests {
        use std::mem;
        use std::sync::mpsc;
        use std::time::Duration;

        use ::windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use ::windows::Win32::System::Threading::GetCurrentThreadId;
        use ::windows::Win32::UI::WindowsAndMessaging::{
            CallWindowProcW, DefWindowProcW, GWLP_USERDATA, GWLP_WNDPROC, GetWindowLongPtrW,
            GetWindowThreadProcessId, IsWindow, PostMessageW, SendMessageW, SetWindowLongPtrW,
            WM_APP, WM_NULL, WNDPROC,
        };

        use super::super::MediaControlManager;

        const DIAGNOSTIC_MESSAGE: u32 = WM_APP + 0x501;

        struct DiagnosticSubclass {
            prev_wnd_proc: isize,
            dispatched_tx: mpsc::Sender<u32>,
        }

        extern "system" fn diagnostic_wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            unsafe {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const DiagnosticSubclass;
                if ptr.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                let subclass = &*ptr;
                if msg == DIAGNOSTIC_MESSAGE {
                    let _ = subclass.dispatched_tx.send(GetCurrentThreadId());
                    return LRESULT(0);
                }
                let prev: WNDPROC = mem::transmute(subclass.prev_wnd_proc);
                CallWindowProcW(prev, hwnd, msg, wparam, lparam)
            }
        }

        fn post_diagnostic_and_wait(hwnd: HWND) -> (u32, Result<u32, mpsc::RecvTimeoutError>) {
            let owner_thread_id = unsafe { GetWindowThreadProcessId(hwnd, None) };
            assert_ne!(
                owner_thread_id, 0,
                "hidden window must have an owning thread"
            );

            let (dispatched_tx, dispatched_rx) = mpsc::channel();
            let prev_wnd_proc = unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) };
            let subclass = Box::into_raw(Box::new(DiagnosticSubclass {
                prev_wnd_proc,
                dispatched_tx,
            }));

            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, subclass as isize);
                SetWindowLongPtrW(
                    hwnd,
                    GWLP_WNDPROC,
                    diagnostic_wnd_proc as *const () as usize as isize,
                );

                let posted = PostMessageW(hwnd, DIAGNOSTIC_MESSAGE, WPARAM(0), LPARAM(0));
                assert!(
                    posted.as_bool(),
                    "PostMessageW to hidden window must succeed"
                );
            }

            let dispatch_result = dispatched_rx.recv_timeout(Duration::from_secs(2));

            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_WNDPROC, prev_wnd_proc);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                // Synchronize with the owning thread so no in-flight wnd_proc still reads `subclass`.
                SendMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
                drop(Box::from_raw(subclass));
            }

            (owner_thread_id, dispatch_result)
        }

        #[test]
        fn hidden_window_dispatches_posted_messages_on_its_owning_thread() {
            let mut manager =
                MediaControlManager::with_name("rune_test_501_dispatch").expect("create manager");
            manager.initialize().expect("initialize manager");

            let caller_thread_id = unsafe { GetCurrentThreadId() };
            let hwnd = manager._dummy_window.handle;
            let (owner_thread_id, dispatch_result) = post_diagnostic_and_wait(hwnd);

            let _ = manager.controls.detach();
            drop(manager);

            let pump_thread_id = dispatch_result.unwrap_or_else(|_| {
                panic!(
                    "diagnostic message posted to hidden window (owner thread {owner_thread_id}) \
                     was never dispatched by its window procedure"
                )
            });

            assert_ne!(
                owner_thread_id, caller_thread_id,
                "hidden window should be owned by its dedicated pump thread"
            );
            assert_eq!(
                pump_thread_id, owner_thread_id,
                "hidden window message must be dispatched on its owning thread"
            );

            let still_exists = unsafe { IsWindow(hwnd).as_bool() };
            assert!(
                !still_exists,
                "hidden window HWND must be destroyed when MediaControlManager is dropped"
            );
        }

        #[test]
        fn multiple_managers_create_and_tear_down_cleanly() {
            let mut first =
                MediaControlManager::with_name("rune_test_501_first").expect("create first");
            first.initialize().expect("initialize first");

            let mut second =
                MediaControlManager::with_name("rune_test_501_second").expect("create second");
            second.initialize().expect("initialize second");

            let first_hwnd = first._dummy_window.handle;
            let second_hwnd = second._dummy_window.handle;
            assert_ne!(first_hwnd, second_hwnd);

            let (first_owner, first_dispatched) = post_diagnostic_and_wait(first_hwnd);
            let (second_owner, second_dispatched) = post_diagnostic_and_wait(second_hwnd);

            assert_ne!(first_owner, second_owner);
            assert_eq!(first_dispatched.expect("first dispatched"), first_owner);
            assert_eq!(second_dispatched.expect("second dispatched"), second_owner);

            let _ = first.controls.detach();
            let _ = second.controls.detach();
            drop(first);
            drop(second);

            unsafe {
                assert!(!IsWindow(first_hwnd).as_bool());
                assert!(!IsWindow(second_hwnd).as_bool());
            }
        }
    }
}
