extern crate proc_mounts;

use proc_mounts::{MountList, SwapList};
use std::io;

fn main() -> io::Result<()> {
    println!("# Active Mounts");
    for mount in MountList::new()?.0 {
        println!("{:#?}", mount);
    }

    println!("# Active Swaps");
    for swap in SwapList::new()?.0 {
        println!("{:#?}", swap);
    }

    Ok(())
}
