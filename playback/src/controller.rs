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
use crate::dummy_souvlaki::{MediaControlEvent, MediaControls, PlatformConfig, SeekDirection};

#[cfg(target_os = "ios")]
use crate::ios_now_playing::MediaControls;
#[cfg(target_os = "ios")]
use souvlaki::{MediaControlEvent, PlatformConfig, SeekDirection};

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

        thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(100));

                // this must be run repeatedly by your program to ensure
                // the Windows event queue is processed by your application
                #[cfg(target_os = "windows")]
                windows::pump_event_queue();
            }
        });

        Ok(())
    }

    pub fn subscribe_controller_events(&self) -> SimpleReceiver<MediaControlEvent> {
        self.event_sender.subscribe()
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
    use std::mem;

    use anyhow::{Context, Result, bail};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GA_ROOT, GetAncestor,
        IsDialogMessageW, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW, TranslateMessage,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_QUIT, WNDCLASSEXW,
    };
    use windows::core::w;

    pub struct DummyWindow {
        pub handle: HWND,
    }

    impl DummyWindow {
        pub fn new() -> Result<DummyWindow> {
            let class_name = w!("SimpleTray");

            let handle_result: Result<HWND> = unsafe {
                let instance =
                    GetModuleHandleW(None).with_context(|| "Unable to get module handle")?;

                let wnd_class = WNDCLASSEXW {
                    cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
                    hInstance: instance,
                    lpszClassName: class_name,
                    lpfnWndProc: Some(Self::wnd_proc),
                    ..Default::default()
                };

                if RegisterClassExW(&wnd_class) == 0 {
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
            };

            handle_result.map(|handle| DummyWindow { handle })
        }
        extern "system" fn wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }

    impl Drop for DummyWindow {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.handle);
            }
        }
    }

    pub fn pump_event_queue() -> bool {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            let mut has_message = PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool();
            while msg.message != WM_QUIT && has_message {
                if !IsDialogMessageW(GetAncestor(msg.hwnd, GA_ROOT), &msg).as_bool() {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                has_message = PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool();
            }

            msg.message == WM_QUIT
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
}
