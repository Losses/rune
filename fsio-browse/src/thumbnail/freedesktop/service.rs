//! The freedesktop thumbnail service, `org.freedesktop.thumbnails.Thumbnailer1`
//! on the session bus. Where a desktop runs one (tumbler on Xfce, for one),
//! it draws thumbnails into the shared cache on request.

use std::collections::HashSet;

use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, Proxy};
use zbus::names::OwnedBusName;

const NAME: &str = "org.freedesktop.thumbnails.Thumbnailer1";
const PATH: &str = "/org/freedesktop/thumbnails/Thumbnailer1";

pub(super) struct ThumbnailService {
    proxy: Proxy<'static>,
    /// Types it draws for `file` URIs.
    types: HashSet<String>,
    /// The cache size folders it fills.
    flavors: Vec<String>,
}

impl ThumbnailService {
    /// The session's thumbnail service, if it runs one or can start one.
    pub(super) fn connect() -> Option<Self> {
        let connection = Connection::session().ok()?;
        let bus = DBusProxy::new(&connection).ok()?;
        let offered = |names: Vec<OwnedBusName>| names.iter().any(|name| name.as_str() == NAME);
        if !bus.list_names().is_ok_and(offered) && !bus.list_activatable_names().is_ok_and(offered)
        {
            return None;
        }
        let proxy = Proxy::new_owned(connection, NAME, PATH, NAME).ok()?;
        let (schemes, mimes): (Vec<String>, Vec<String>) = proxy.call("GetSupported", &()).ok()?;
        let types = schemes
            .into_iter()
            .zip(mimes)
            .filter(|(scheme, _)| scheme == "file")
            .map(|(_, mime)| mime)
            .collect();
        let flavors = proxy
            .call("GetFlavors", &())
            .unwrap_or_else(|_| vec!["normal".to_owned(), "large".to_owned()]);
        Some(Self {
            proxy,
            types,
            flavors,
        })
    }

    pub(super) fn handles(&self, mime: &str) -> bool {
        self.types.contains(mime)
    }

    /// The cache folder the service fills that best holds `wanted`: that one
    /// if offered, else the largest offered.
    pub(super) fn flavor(&self, wanted: &str, order: &[&str]) -> Option<String> {
        if self.flavors.iter().any(|flavor| flavor == wanted) {
            return Some(wanted.to_owned());
        }
        order
            .iter()
            .rev()
            .find(|flavor| self.flavors.iter().any(|offered| offered == *flavor))
            .map(|flavor| (*flavor).to_owned())
    }

    /// Ask for a thumbnail of `uri` in the `flavor` cache folder. The service
    /// writes it there when done.
    pub(super) fn queue(&self, uri: &str, mime: &str, flavor: &str) -> bool {
        self.proxy
            .call::<_, _, u32>(
                "Queue",
                &(vec![uri], vec![mime], flavor, "foreground", 0u32),
            )
            .is_ok()
    }
}
