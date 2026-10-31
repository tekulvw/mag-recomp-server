mod navmesh_spawn_exit_cli;
use mag_gameplay::navmesh::{self, Artifact, BakeInput, NavMesh};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::Path,
    time::Instant,
};
fn read(path: &str, max: u64) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if fs::metadata(path)?.len() > max {
        return Err("input size limit".into());
    }
    Ok(fs::read(path)?)
}
fn point(value: &str) -> Result<[f64; 3], Box<dyn std::error::Error>> {
    let values: Vec<f64> = value.split(',').map(str::parse).collect::<Result<_, _>>()?;
    Ok(values.try_into().map_err(|_| "point requires x,y,z")?)
}
fn write(
    path: Option<&String>,
    data: &impl serde::Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec(data)?;
    if let Some(path) = path {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(Path::new(path))?;
        f.write_all(&bytes)?;
        f.write_all(b"\n")?;
    } else {
        io::stdout().write_all(&bytes)?;
        println!();
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or("expected bake|query|inspect")?;
    let mut flags = BTreeMap::new();
    while let Some(key) = args.next() {
        if !key.starts_with("--") || flags.contains_key(&key) {
            return Err("invalid/duplicate flag".into());
        }
        flags.insert(key, args.next().ok_or("flag requires value")?);
    }
    let get = |key: &str| {
        flags
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {key}"))
    };
    match command.as_str() {
        "bake" => {
            if flags
                .keys()
                .any(|k| !["--input", "--output"].contains(&k.as_str()))
            {
                return Err("unknown bake flag".into());
            }
            let input: BakeInput =
                serde_json::from_slice(&read(get("--input")?, 128 * 1024 * 1024)?)?;
            let began = Instant::now();
            let artifact = navmesh::bake(input)?;
            write(Some(&get("--output")?.to_string()), &artifact)?;
            eprintln!(
                "{}",
                json!({"op":"bake_complete","elapsed_ms":began.elapsed().as_millis(),"detour_bytes":artifact.detour_data.len(),"world_sha256":artifact.world_sha256,"detour_sha256":artifact.detour_sha256})
            );
        }
        "search-spawn-exits" => {
            if flags.keys().any(|k| {
                ![
                    "--mesh",
                    "--collision-world",
                    "--parameters",
                    "--output",
                    "--fall-damage-rules",
                    "--health",
                ]
                .contains(&k.as_str())
            }) {
                return Err("unknown search-spawn-exits flag".into());
            }
            navmesh_spawn_exit_cli::run(
                get("--collision-world")?,
                get("--mesh")?,
                get("--parameters")?,
                get("--output")?,
                get("--fall-damage-rules")?,
                get("--health")?.parse()?,
            )?;
        }
        "replay-drop" => {
            if flags.keys().any(|k| {
                !["--mesh", "--collision-world", "--fixture", "--output"].contains(&k.as_str())
            }) {
                return Err("unknown replay-drop flag".into());
            }
            navmesh_spawn_exit_cli::replay_case(
                get("--collision-world")?,
                get("--mesh")?,
                get("--fixture")?,
                get("--output")?,
            )?;
        }
        "add-drop" => {
            if flags.keys().any(|k| {
                ![
                    "--mesh",
                    "--collision-world",
                    "--launch",
                    "--landing",
                    "--speed",
                    "--id",
                    "--output",
                ]
                .contains(&k.as_str())
            }) {
                return Err("unknown add-drop flag".into());
            }
            let mut artifact: Artifact =
                serde_json::from_slice(&read(get("--mesh")?, navmesh::MAX_ARTIFACT_JSON_BYTES)?)?;
            let world_path = get("--collision-world")?;
            let hash = navmesh::sha256(&read(world_path, 128 * 1024 * 1024)?);
            let world =
                mag_gameplay::collision::CollisionWorld::load(world_path, Default::default())?;
            let mesh = NavMesh::load(artifact.clone(), &hash, &world.metadata().map_name)?;
            mesh.validate_coverage(&world)?;
            let link = navmesh::DropLink::certify(
                &world,
                &hash,
                get("--id")?.parse()?,
                point(get("--launch")?)?,
                point(get("--landing")?)?,
                get("--speed")?.parse()?,
            )?;
            artifact.drop_links.push(link);
            let _ = NavMesh::load(artifact.clone(), &hash, &world.metadata().map_name)?;
            write(Some(&get("--output")?.to_string()), &artifact)?;
        }
        "query" | "inspect" | "component" => {
            if flags.keys().any(|k| {
                ![
                    "--mesh",
                    "--start",
                    "--goal",
                    "--goal-radius",
                    "--output",
                    "--repeat",
                    "--fall-damage-rules",
                    "--health",
                ]
                .contains(&k.as_str())
            }) {
                return Err("unknown query flag".into());
            }
            let load_began = Instant::now();
            let artifact: Artifact =
                serde_json::from_slice(&read(get("--mesh")?, navmesh::MAX_ARTIFACT_JSON_BYTES)?)?;
            let world = artifact.world_sha256.clone();
            let map = artifact.map_name.clone();
            let mesh = NavMesh::load(artifact, &world, &map)?;
            let load_us = load_began.elapsed().as_micros();
            if command == "component" {
                let start = point(get("--start")?)?;
                let goal = point(get("--goal")?)?;
                let diagnostic = mesh.component_diagnostic(start, goal)?;
                write(flags.get("--output"), &diagnostic)?;
            } else if command == "inspect" {
                let mut value = serde_json::to_value(mesh.artifact())?;
                value.as_object_mut().unwrap().remove("detour_data");
                value["detour_bytes"] = json!(mesh.artifact().detour_data.len());
                write(flags.get("--output"), &value)?;
            } else {
                let start = point(get("--start")?)?;
                let goal = point(get("--goal")?)?;
                let radius = get("--goal-radius")?.parse()?;
                let repeat: usize = flags.get("--repeat").map_or(Ok(1), |v| v.parse())?;
                if !(1..=100000).contains(&repeat) {
                    return Err("repeat must be 1..100000".into());
                }
                let rules = flags
                    .get("--fall-damage-rules")
                    .map(mag_gameplay::fall_damage::Rules::load)
                    .transpose()?;
                let health = flags.get("--health").map_or(Ok(400u16), |v| v.parse())?;
                let query = || {
                    if let Some(rules) = &rules {
                        mesh.route_with_drops(start, goal, radius, rules, health)
                    } else {
                        mesh.route(start, goal, radius)
                    }
                };
                let began = Instant::now();
                let mut timings = Vec::with_capacity(repeat);
                let mut route = query()?;
                timings.push(route.query_us);
                for _ in 1..repeat {
                    route = query()?;
                    timings.push(route.query_us);
                }
                timings.sort_unstable();
                eprintln!(
                    "{}",
                    json!({"op":"query_benchmark","repeat":repeat,"total_us":began.elapsed().as_micros(),"mean_us":began.elapsed().as_secs_f64()*1e6/repeat as f64,"p50_us":timings[(repeat-1)/2],"p95_us":timings[((repeat-1)*95)/100],"max_us":timings[repeat-1],"load_us":load_us,"status":route.status})
                );
                write(flags.get("--output"), &route)?;
            }
        }
        _ => return Err("expected bake|query|inspect".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn oversized_serialized_artifact_is_rejected_before_reading() {
        let path = std::env::temp_dir().join(format!(
            "mag-navmesh-size-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.set_len(super::navmesh::MAX_ARTIFACT_JSON_BYTES + 1)
            .unwrap();
        let result = super::read(
            path.to_str().unwrap(),
            super::navmesh::MAX_ARTIFACT_JSON_BYTES,
        );
        std::fs::remove_file(path).unwrap();
        assert_eq!(result.unwrap_err().to_string(), "input size limit");
    }
}
