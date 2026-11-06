//! LOCAL DIAGNOSTIC: free-flow walking distance from each spawn to each drop.
//! usage: exit_floor WORLD.json MESH.mag-nav SPAWNS.jsonl
use mag_gameplay::{
    collision::{CollisionLimits, CollisionWorld},
    navmesh::{self, Artifact, NavMesh, Status},
};
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let world = CollisionWorld::load(&args[1], CollisionLimits::default())?;
    let hash = navmesh::sha256(&fs::read(&args[1])?);
    let art: Artifact = serde_json::from_slice(&fs::read(&args[2])?)?;
    let links = art.drop_links.clone();
    let mesh = NavMesh::load(art, &hash, &world.metadata().map_name)?;
    for line in fs::read_to_string(&args[3])?.lines() {
        let v: Vec<f64> = serde_json::from_str(line)?;
        let start = [v[3], v[4], v[5]];
        let mut row = Vec::new();
        for l in &links {
            if let Ok(r) = mesh.route(start, l.launch, 0.75)
                && r.status == Status::Complete
            {
                row.push(format!("{}:{:.1}", l.id, r.length_m));
            }
        }
        println!("{} team={} spawn={} {}", v[0], v[1], v[2], row.join(" "));
    }
    Ok(())
}
