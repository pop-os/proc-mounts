//! Provides easy access to data from the `/proc/swaps` and `/proc/mounts` files.
//!
//! ```rust,no_run
//! extern crate proc_mounts;
//!
//! use proc_mounts::{MountList, SwapList};
//! use std::io;
//!
//! fn main() -> io::Result<()> {
//!     println!("# Active Mounts");
//!     for mount in MountList::new()?.0 {
//!         println!("{:#?}", mount);
//!     }
//!
//!     println!("# Active Swaps");
//!     for swap in SwapList::new()?.0 {
//!         println!("{:#?}", swap);
//!     }
//!
//!     Ok(())
//! }
//! ```

#[macro_use]
extern crate lazy_static;

mod mounts;
mod swaps;

use std::collections::hash_map::DefaultHasher;

use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

pub use self::mounts::*;
pub use self::swaps::*;

lazy_static! {
    /// Static list of mounts that is dynamically updated in the background.
    pub static ref MOUNTS: Arc<RwLock<MountList>> = {
        let mounts = Arc::new(RwLock::new(MountList::new().unwrap()));
        watch_and_set(mounts.clone(), "/proc/mounts", || MountList::new().ok());
        mounts
    };
}

lazy_static! {
    /// Static list of swap points that is dynamically updated in the background.
    pub static ref SWAPS: Arc<RwLock<SwapList>> = {
        let swaps = Arc::new(RwLock::new(SwapList::new().unwrap()));
        watch_and_set(swaps.clone(), "/proc/swaps", || SwapList::new().ok());
        swaps
    };
}

fn watch_and_set<T: 'static + Send + Sync>(
    swaps: Arc<RwLock<T>>,
    file: &'static str,
    create_new: fn() -> Option<T>
) {
    thread::spawn(move || {
        let buffer: &mut [u8] = &mut [0u8; 8 * 1024];
        let modified = &mut get_file_hash(file, buffer).expect("hash could not be obtained");

        loop {
            thread::sleep(Duration::from_secs(1));
            modify_if_changed(&swaps, modified, buffer, file, create_new);
        }
    });
}

fn modify_if_changed<T: 'static + Send + Sync>(
    swaps: &Arc<RwLock<T>>,
    modified: &mut u64,
    buffer: &mut [u8],
    file: &'static str,
    create_new: fn() -> Option<T>
) {
    if let Ok(new_modified) = get_file_hash(file, buffer) {
        if new_modified != *modified {
            *modified = new_modified;
            if let Ok(ref mut swaps) = swaps.write() {
                if let Some(new_swaps) = create_new() {
                    **swaps = new_swaps;
                }
            }
        }
    }
}

fn get_file_hash<P: AsRef<Path>>(path: P, buffer: &mut [u8]) -> io::Result<u64> {
    let mut file = open(path)?;
    let hasher = &mut DefaultHasher::new();
    while let Ok(read) = file.read(buffer) {
        if read == 0 {
            break;
        }
        buffer[..read].hash(hasher);
    }
    Ok(hasher.finish())
}

fn open<P: AsRef<Path>>(path: P) -> io::Result<File> {
    File::open(&path).map_err(|why| io::Error::new(
        io::ErrorKind::Other,
        format!("unable to open file at {:?}: {}", path.as_ref(), why)
    ))
}
