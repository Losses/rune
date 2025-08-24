#[cfg(target_os = "windows")]
use std::ffi::c_void;
use std::io::Write;
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

#[cfg(not(target_os = "android"))]
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
        #[cfg(not(any(target_os = "windows")))]
        let hwnd = None;

        #[cfg(target_os = "windows")]
        let (hwnd, dummy_window) = {
            let dummy_window = windows::DummyWindow::new()?;
            let handle = dummy_window.handle.0 as *mut c_void;
            (Some(handle), dummy_window)
        };

        let config = PlatformConfig {
            dbus_name: "rune_player",
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
                std::thread::sleep(std::time::Duration::from_millis(100));

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
        MediaControlEvent::Seek(direction) => {
            let seek_seconds: f64 = match direction {
                SeekDirection::Forward => 10.0,
                SeekDirection::Backward => -10.0,
            };

            let current_position = player.lock().await.get_status().position;

            player
                .lock()
                .await
                .seek(current_position.as_millis() as f64 + seek_seconds * 1000.0);
        }
        MediaControlEvent::SetPosition(position) => {
            player.lock().await.seek(position.0.as_millis() as f64)
        }
        _ => debug!("Unhandled media control event: {event:?}"),
    }

    Ok(())
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
    use windows::core::{PCWSTR, w};

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
                    lpszClassName: PCWSTR::from(class_name),
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
