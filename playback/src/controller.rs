#[cfg(target_os = "windows")]
use std::ffi::c_void;
use std::{sync::Arc, thread};

use anyhow::{bail, Error, Result};
use log::{debug, error, info};
use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig};
use tokio::sync::{broadcast, Mutex};

use crate::player::{PlaybackState, Player};

pub struct MediaControlManager {
    pub controls: MediaControls,
    event_sender: broadcast::Sender<MediaControlEvent>,
    #[cfg(target_os = "windows")]
    dummy_window: windows::DummyWindow,
}

impl MediaControlManager {
    pub fn new() -> Result<Self> {
        #[cfg(not(target_os = "windows"))]
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
            Err(e) => bail!(Error::msg(format!("{:?}", e))),
        };

        let (event_sender, _) = broadcast::channel(32);

        Ok(Self {
            controls,
            event_sender,
            #[cfg(target_os = "windows")]
            dummy_window,
        })
    }

    pub fn initialize(&mut self) -> Result<()> {
        info!("Initializing media controls");

        let event_sender = self.event_sender.clone();
        let request = self.controls.attach(move |event: MediaControlEvent| {
            let event_sender = event_sender.clone();
            thread::spawn(move || {
                if let Err(e) = event_sender.send(event) {
                    error!("Error sending media control event: {:?}", e);
                }
            });
        });

        match request {
            Ok(x) => x,
            Err(e) => bail!(Error::msg(format!("{:?}", e))),
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

    pub fn subscribe_controller_events(&self) -> broadcast::Receiver<MediaControlEvent> {
        self.event_sender.subscribe()
    }
}

pub async fn handle_media_control_event(
    player: &Arc<Mutex<Player>>,
    event: MediaControlEvent,
) -> Result<()> {
    debug!("Received media control event: {:?}", event);

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
                souvlaki::SeekDirection::Forward => 10.0,
                souvlaki::SeekDirection::Backward => -10.0,
            };

            let current_position = player.lock().await.current_status.lock().unwrap().position;

            player
                .lock()
                .await
                .seek(current_position.as_millis() as f64 + seek_seconds * 1000.0);
        }
        MediaControlEvent::SetPosition(position) => {
            player.lock().await.seek(position.0.as_millis() as f64)
        }
        _ => debug!("Unhandled media control event: {:?}", event),
    }

    Ok(())
}

#[cfg(target_os = "windows")]
mod windows {
    use std::io::Error;
    use std::mem;

    use anyhow::{bail, Context, Result};

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetAncestor,
        IsDialogMessageW, PeekMessageW, RegisterClassExW, TranslateMessage, GA_ROOT, MSG,
        PM_REMOVE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_QUIT, WNDCLASSEXW,
    };

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
