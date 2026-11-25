mod navmesh_spawn_exit_cli;
use mag_gameplay::navmesh::{
    self, Artifact, BakeInput, Layout, MovementProfile, NavMesh, ReferenceManifest,
    ReferenceNavMesh,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
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

fn read_reference_file(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > max {
        return Err(io::Error::other("invalid reference bundle file or size"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::other("reference bundle file size limit"));
    }
    Ok(bytes)
}

fn open_reference(
    path: &Path,
    rules: Option<&MovementProfile>,
) -> Result<(ReferenceManifest, ReferenceNavMesh), Box<dyn std::error::Error>> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err("expected a reference bundle directory".into());
    }
    let manifest: ReferenceManifest = serde_json::from_slice(&read_reference_file(
        &path.join("manifest.json"),
        64 * 1024,
    )?)?;
    if !(100..=navmesh::MAX_REFERENCE_BYTES).contains(&manifest.detour_bytes) {
        return Err("invalid reference Detour size".into());
    }
    let data = read_reference_file(&path.join("detour.bin"), manifest.detour_bytes)?;
    let mesh = match rules {
        Some(rules) => ReferenceNavMesh::load_with_movement_profile(
            &manifest,
            data,
            &manifest.world_sha256,
            &manifest.map_name,
            rules,
        ),
        None => ReferenceNavMesh::load(&manifest, data, &manifest.world_sha256, &manifest.map_name),
    }?;
    Ok((manifest, mesh))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or("expected a navmesh command")?;
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
    let coverage = flags.get("--collision-coverage").map(String::as_str);
    let movement_profile = || {
        flags
            .get("--movement-profile")
            .map(MovementProfile::load)
            .transpose()
    };
    match command.as_str() {
        "bake-regional-walk" => {
            if flags
                .keys()
                .any(|key| !["--input", "--output", "--coverage"].contains(&key.as_str()))
            {
                return Err("unknown regional walking bake flag".into());
            }
            let input = serde_json::from_slice(&read(get("--input")?, 128 * 1024 * 1024)?)?;
            let coverage = navmesh::regional::Coverage::load(get("--coverage")?)?;
            let artifact = navmesh::regional::bake(input, &coverage)?;
            write(
                Some(flags.get("--output").ok_or("missing --output")?),
                &artifact,
            )?;
        }
        "query-regional-walk" => {
            if flags.keys().any(|key| {
                ![
                    "--mesh",
                    "--collision-world",
                    "--coverage",
                    "--start",
                    "--goal",
                    "--goal-radius",
                    "--output",
                ]
                .contains(&key.as_str())
            }) {
                return Err("unknown regional walking query flag".into());
            }
            let artifact =
                serde_json::from_slice(&read(get("--mesh")?, navmesh::MAX_ARTIFACT_JSON_BYTES)?)?;
            let world = mag_gameplay::collision::CollisionWorld::load(
                get("--collision-world")?,
                Default::default(),
            )?;
            let coverage = navmesh::regional::Coverage::load(get("--coverage")?)?;
            let mesh = navmesh::regional::load(artifact, &world, coverage)?;
            let route = mesh.route(
                point(get("--start")?)?,
                point(get("--goal")?)?,
                get("--goal-radius")?.parse()?,
            )?;
            write(flags.get("--output"), &route)?;
        }
        "bake-reference" => {
            if flags
                .keys()
                .any(|key| !["--input", "--output", "--movement-profile"].contains(&key.as_str()))
            {
                return Err("unknown reference bake flag".into());
            }
            let output = Path::new(get("--output")?);
            if output.exists() {
                return Err("reference output directory already exists".into());
            }
            let input: BakeInput =
                serde_json::from_slice(&read(get("--input")?, 128 * 1024 * 1024)?)?;
            let began = Instant::now();
            let (manifest, data) = match movement_profile()?.as_ref() {
                Some(rules) => navmesh::bake_reference_with_movement_profile(input, rules),
                None => navmesh::bake_reference(input),
            }?;
            fs::create_dir(output)?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output.join("detour.bin"))?
                .write_all(&data)?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output.join("manifest.json"))?;
            serde_json::to_writer(&mut file, &manifest)?;
            file.write_all(b"\n")?;
            eprintln!(
                "{}",
                json!({"op":"reference_bake_complete","elapsed_ms":began.elapsed().as_millis(),
                    "detour_bytes":manifest.detour_bytes,"world_sha256":manifest.world_sha256,
                    "detour_sha256":manifest.detour_sha256})
            );
        }
        "inspect-reference" | "query-reference" => {
            let allowed: &[&str] = if command == "inspect-reference" {
                &["--mesh", "--output", "--movement-profile"]
            } else {
                &[
                    "--mesh",
                    "--start",
                    "--goal",
                    "--goal-radius",
                    "--output",
                    "--movement-profile",
                ]
            };
            if flags.keys().any(|key| !allowed.contains(&key.as_str())) {
                return Err("unknown reference inspection/query flag".into());
            }
            let (manifest, mesh) =
                open_reference(Path::new(get("--mesh")?), movement_profile()?.as_ref())?;
            if command == "inspect-reference" {
                let (tiles, max_polygons) = mesh.tile_stats()?;
                let mut report = serde_json::to_value(manifest)?;
                report["tiles"] = json!(tiles);
                report["max_polygons_per_tile"] = json!(max_polygons);
                write(flags.get("--output"), &report)?;
            } else {
                let route = mesh.route(
                    point(get("--start")?)?,
                    point(get("--goal")?)?,
                    get("--goal-radius")?.parse()?,
                )?;
                write(flags.get("--output"), &route)?;
            }
        }
        "bake" => {
            if flags.keys().any(|k| {
                !["--input", "--output", "--movement-profile", "--layout"].contains(&k.as_str())
            }) {
                return Err("unknown bake flag".into());
            }
            let layout = match flags.get("--layout").map(String::as_str) {
                None | Some("auto") => Layout::Auto,
                Some("wide") => Layout::Wide,
                Some(_) => return Err("--layout must be auto or wide".into()),
            };
            let mut input: serde_json::Value =
                serde_json::from_slice(&read(get("--input")?, 128 * 1024 * 1024)?)?;
            // prepare.py records the character movement layers it kept.
            let layers = match input
                .as_object_mut()
                .and_then(|o| o.remove("character_movement_layers"))
            {
                None => None,
                Some(v) => Some(
                    v.as_u64()
                        .and_then(|v| u32::try_from(v).ok())
                        .ok_or("invalid character_movement_layers")?,
                ),
            };
            let input: BakeInput = serde_json::from_value(input)?;
            let began = Instant::now();
            let grid_tiles = navmesh::grid_tiles(&input.bounds);
            let mut artifact =
                navmesh::bake_with_layout(input, movement_profile()?.as_ref(), layout)?;
            artifact.character_movement_layers = layers;
            write(Some(&get("--output")?.to_string()), &artifact)?;
            eprintln!(
                "{}",
                json!({"op":"bake_complete","elapsed_ms":began.elapsed().as_millis(),"format":artifact.format,"grid_tiles":grid_tiles,"detour_bytes":artifact.detour_data.len(),"world_sha256":artifact.world_sha256,"detour_sha256":artifact.detour_sha256})
            );
        }
        "search-spawn-exits" => {
            if flags.keys().any(|k| {
                ![
                    "--mesh",
                    "--collision-world",
                    "--collision-coverage",
                    "--parameters",
                    "--output",
                    "--fall-damage-rules",
                    "--health",
                    "--movement-profile",
                ]
                .contains(&k.as_str())
            }) {
                return Err("unknown search-spawn-exits flag".into());
            }
            navmesh_spawn_exit_cli::run(
                get("--collision-world")?,
                coverage,
                get("--mesh")?,
                get("--parameters")?,
                get("--output")?,
                get("--fall-damage-rules")?,
                get("--health")?.parse()?,
                movement_profile()?.as_ref(),
            )?;
        }
        "replay-drop" => {
            if flags.keys().any(|k| {
                ![
                    "--mesh",
                    "--collision-world",
                    "--collision-coverage",
                    "--fixture",
                    "--output",
                    "--movement-profile",
                ]
                .contains(&k.as_str())
            }) {
                return Err("unknown replay-drop flag".into());
            }
            navmesh_spawn_exit_cli::replay_case(
                get("--collision-world")?,
                coverage,
                get("--mesh")?,
                get("--fixture")?,
                get("--output")?,
                movement_profile()?.as_ref(),
            )?;
        }
        "add-drop" => {
            if flags.keys().any(|k| {
                ![
                    "--mesh",
                    "--collision-world",
                    "--collision-coverage",
                    "--launch",
                    "--landing",
                    "--speed",
                    "--id",
                    "--output",
                    "--movement-profile",
                ]
                .contains(&k.as_str())
            }) {
                return Err("unknown add-drop flag".into());
            }
            let mut artifact: Artifact =
                serde_json::from_slice(&read(get("--mesh")?, navmesh::MAX_ARTIFACT_JSON_BYTES)?)?;
            let world_path = get("--collision-world")?;
            let hash = navmesh::sha256(&read(world_path, 128 * 1024 * 1024)?);
            let world = navmesh_spawn_exit_cli::load_world(world_path, coverage)?;
            let rules = movement_profile()?;
            let map = world.metadata().map_name.clone();
            let mesh =
                navmesh_spawn_exit_cli::load_mesh(artifact.clone(), &hash, &map, rules.as_ref())?;
            let world =
                world.with_character_movement_layers(mesh.artifact().character_movement_layers);
            mesh.validate_coverage(&world)?;
            let link = navmesh::DropLink::certify_with_body(
                &world,
                &hash,
                get("--id")?.parse()?,
                point(get("--launch")?)?,
                point(get("--landing")?)?,
                get("--speed")?.parse()?,
                mesh.body(),
            )?;
            artifact.drop_links.push(link);
            let _ =
                navmesh_spawn_exit_cli::load_mesh(artifact.clone(), &hash, &map, rules.as_ref())?;
            write(Some(&get("--output")?.to_string()), &artifact)?;
        }
        "goals" => {
            if flags.keys().any(|key| {
                ![
                    "--mesh",
                    "--collision-world",
                    "--collision-coverage",
                    "--goal",
                    "--goal-radius",
                    "--output",
                    "--movement-profile",
                ]
                .contains(&key.as_str())
            }) {
                return Err("unknown goals flag".into());
            }
            let artifact: Artifact =
                serde_json::from_slice(&read(get("--mesh")?, navmesh::MAX_ARTIFACT_JSON_BYTES)?)?;
            let world = navmesh_spawn_exit_cli::load_world(get("--collision-world")?, coverage)?;
            let mesh = match movement_profile()?.as_ref() {
                Some(rules) => NavMesh::load_with_movement_profile(
                    artifact,
                    world.source_sha256(),
                    &world.metadata().map_name,
                    rules,
                ),
                None => NavMesh::load(artifact, world.source_sha256(), &world.metadata().map_name),
            }?;
            mesh.validate_coverage(&world)?;
            let mut goals =
                mesh.goal_candidates(point(get("--goal")?)?, get("--goal-radius")?.parse()?)?;
            goals.sort_by_key(|(component, _)| *component);
            let candidates: Vec<_> = goals
                .into_iter()
                .map(|(component, point)| {
                    json!({
                        "component": component, "point": point
                    })
                })
                .collect();
            write(
                flags.get("--output"),
                &json!({
                    "map": world.metadata().map_name,
                    "world_sha256": world.source_sha256(),
                    "detour_sha256": mesh.artifact().detour_sha256,
                    "candidates": candidates,
                }),
            )?;
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
                    "--movement-profile",
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
            let mesh = match movement_profile()?.as_ref() {
                Some(rules) => NavMesh::load_with_movement_profile(artifact, &world, &map, rules),
                None => NavMesh::load(artifact, &world, &map),
            }?;
            let load_us = load_began.elapsed().as_micros();
            if command == "component" {
                let start = point(get("--start")?)?;
                let goal = point(get("--goal")?)?;
                let diagnostic = mesh.component_diagnostic(start, goal)?;
                write(flags.get("--output"), &diagnostic)?;
            } else if command == "inspect" {
                let mut value = serde_json::to_value(mesh.artifact())?;
                value.as_object_mut().unwrap().remove("detour_data");
                value["detour_bytes"] = json!(mesh.detour_bytes());
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
        _ => return Err("unknown navmesh command".into()),
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
