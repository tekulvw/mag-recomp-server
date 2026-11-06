//! LOCAL DIAGNOSTIC: can a spawn walk to its goal without a drop? Where does
//! the walkable island end?  usage: walk_out WORLD MESH "x,y,z" "x,y,z"...pairs
use mag_gameplay::{
    collision::{CollisionLimits, CollisionWorld},
    navmesh::{self, Artifact, NavMesh},
};
use std::fs;
fn p(s: &str) -> [f64; 3] {
    let v: Vec<f64> = s.split(',').map(|x| x.parse().unwrap()).collect();
    [v[0], v[1], v[2]]
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let world = CollisionWorld::load(&a[1], CollisionLimits::default())?;
    let hash = navmesh::sha256(&fs::read(&a[1])?);
    let art: Artifact = serde_json::from_slice(&fs::read(&a[2])?)?;
    let mesh = NavMesh::load(art, &hash, &world.metadata().map_name)?;
    for pair in a[3..].chunks(2) {
        let (s, g) = (p(&pair[0]), p(&pair[1]));
        let r = mesh.route(s, g, 5.)?;
        let end = r.waypoints.last().copied().unwrap_or(s);
        println!(
            "start={:?} comp={:?} goal comp={:?} status={:?} length={:.1} end={:?}",
            s,
            mesh.component_id(s).ok(),
            mesh.component_id(g).ok(),
            r.status,
            r.length_m,
            end.map(|v| (v * 10.).round() / 10.)
        );
    }
    Ok(())
}
