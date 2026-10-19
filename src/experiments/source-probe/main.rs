#![forbid(unsafe_code)]
use anyhow::Result;
use rsmpv::Mpv;
use std::time::{Duration, Instant};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let mpv = Mpv::builder()?
        .set_property("vo", "null")?
        .set_property("ao", "null")?
        .set_property("keep-open", true)?
        .build()?;
    for source in [&args[1], &args[2], &args[1]] {
        println!(
            "load {source}: {:?}",
            mpv.command_ret(&["loadfile", source, "replace"])
        );
        println!(
            "playlist immediately {:?}",
            mpv.get_property::<rsmpv::Node>("playlist")
        );
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(800) {
        while let Some(event) = mpv.poll_event() {
            println!("{:?}: {event:?}", start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}
