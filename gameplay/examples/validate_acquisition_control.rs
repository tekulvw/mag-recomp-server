//! Offline external controller consumer. No engine, listener or account store.
use mag_gameplay::{
    acquisition_control::{Catalog, ObservedState},
    spawn_catalog::SpawnCatalog,
};
use std::{collections::BTreeMap, io::Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [spawn_path, controller_path, observations_path] = args.as_slice() else {
        return Err("usage: validate_acquisition_control SPAWNS CONTROLLER OBSERVATIONS".into());
    };
    let spawns = SpawnCatalog::load(spawn_path)?;
    let controller = Catalog::load(controller_path, &spawns)?;
    let mut raw = String::new();
    std::fs::File::open(observations_path)?
        .take(65_537)
        .read_to_string(&mut raw)?;
    if raw.len() > 65_536 {
        return Err("observations exceed bound".into());
    }
    let observations: BTreeMap<u32, ObservedState> = serde_json::from_str(&raw)?;
    println!(
        "{}",
        serde_json::to_string(&controller.plan(&observations)?)?
    );
    Ok(())
}
