//! Prints what the file dialog's navigation pane would list on this device,
//! and the thumbnail size of any files given as arguments.

use std::env;
use std::path::Path;

use fsio_browse::{ThumbnailSize, Thumbnailer, system_places};

fn main() {
    let places = system_places();
    println!("Pinned:");
    for place in &places.pinned {
        println!("  {:?} {:?}", place.name, place.location.path);
    }
    println!("Folders:");
    for place in &places.folders {
        println!("  {:?} {:?}", place.name, place.location.path);
    }
    println!("Volumes:");
    for volume in &places.volumes {
        println!(
            "  {:?} {:?} {:?} {:?}",
            volume.kind, volume.label, volume.location.path, volume.space
        );
    }
    let thumbnailer = Thumbnailer::new();
    for file in env::args().skip(1) {
        match thumbnailer.file(Path::new(&file), ThumbnailSize(256)) {
            Ok(Some(thumbnail)) => println!("{file}: {}x{}", thumbnail.width, thumbnail.height),
            Ok(None) => println!("{file}: no thumbnail"),
            Err(error) => println!("{file}: {error:#}"),
        }
    }
}
