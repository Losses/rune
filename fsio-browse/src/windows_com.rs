use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{IShellItem, SIGDN};
use windows::core::{Error, HRESULT};

/// COM on the current thread for as long as this lives. Shell items and
/// their thumbnail handlers expect a single-threaded apartment.
pub(crate) struct Apartment {
    entered: bool,
}

impl Apartment {
    pub(crate) fn enter() -> Self {
        // SAFETY: the reserved pointer is null and the flags are valid.
        let result =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        // S_OK and S_FALSE both must be balanced by CoUninitialize. A thread
        // already in another apartment keeps it, which serves as well and is
        // not this guard's to leave.
        Self {
            entered: result.is_ok(),
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.entered {
            // SAFETY: balances the successful CoInitializeEx in `enter`.
            unsafe { CoUninitialize() };
        }
    }
}

/// One of a shell item's names, as text.
pub(crate) fn display_name(item: &IShellItem, kind: SIGDN) -> windows::core::Result<String> {
    // SAFETY: GetDisplayName returns a CoTaskMem string this function owns
    // and frees once copied.
    unsafe {
        let name = item.GetDisplayName(kind)?;
        let text = name.to_string();
        CoTaskMemFree(Some(name.0 as *const _));
        text.map_err(|_| Error::from_hresult(HRESULT::from_win32(1113)))
    }
}
