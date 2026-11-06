use mag_gameplay::collision::{CollisionLimits, CollisionWorld};
use mag_gameplay::telemetry::{TickRecorder, TickSample};
use mag_gameplay::{
    Engine, EngineResources, IO_DEADLINE, MAX_LOG, Response, parse_request, read_line,
};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn parse_application_id(value: &str) -> io::Result<u32> {
    match value {
        "20230" => Ok(20230),
        "20231" => Ok(20231),
        _ => Err(io::Error::other("--application-id requires 20230 or 20231")),
    }
}

fn validate_runtime_paths(
    application_id: Option<u32>,
    paths: &[(&str, Option<&Path>)],
) -> io::Result<()> {
    if application_id != Some(20231) {
        return Ok(());
    }
    for (flag, path) in paths {
        match path {
            Some(path) if path.is_absolute() => {}
            _ => {
                return Err(io::Error::other(format!(
                    "retail startup requires an explicit absolute {flag}"
                )));
            }
        }
    }
    Ok(())
}

fn validate_item_catalog_links(items: &Path, linked: &[(&str, &Path, &str)]) -> io::Result<()> {
    let digest = format!("{:x}", Sha256::digest(std::fs::read(items)?));
    for (name, path, pointer) in linked {
        let metadata: serde_json::Value = serde_json::from_reader(std::fs::File::open(path)?)
            .map_err(|_| io::Error::other(format!("invalid {name} metadata")))?;
        if metadata
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            != Some(digest.as_str())
        {
            return Err(io::Error::other(format!(
                "{name} does not match the selected retail item catalog"
            )));
        }
    }
    Ok(())
}

type LogWriter = Box<dyn std::io::Write + Send>;

/// Open a NEW log file, segmented when a rotation policy is configured.
fn open_log(
    path: &std::path::Path,
    rotation: Option<mag_gameplay::rotating_log::RotationPolicy>,
) -> io::Result<LogWriter> {
    Ok(match rotation {
        Some(policy) => Box::new(mag_gameplay::rotating_log::RotatingLog::create(
            path.to_path_buf(),
            policy,
        )?),
        None => Box::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?,
        ),
    })
}

/// One delay for both teams, or "TEAM0,TEAM1". Range checks happen in the catalog.
fn parse_team_delays(value: &str) -> Option<[u32; 2]> {
    let parts: Vec<u32> = value
        .split(',')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [both] => Some([*both, *both]),
        [team0, team1] => Some([*team0, *team1]),
        _ => None,
    }
}
fn main() -> io::Result<()> {
    run(std::env::args().skip(1))
}

fn run(args: impl IntoIterator<Item = String>) -> io::Result<()> {
    let mut application_id: Option<u32> = None;
    let mut validate_catalogs = false;
    let mut headless_sim_seconds: Option<u64> = None;
    let mut bind = SocketAddr::from(([127, 0, 0, 1], 31000));
    let mut log: Option<PathBuf> = None;
    let mut log_segment_bytes: Option<u64> = None;
    let mut log_retention_s: Option<u64> = None;
    let mut telemetry_path: Option<PathBuf> = None;
    let mut tick_budget_us = None;
    let mut collision_path: Option<PathBuf> = None;
    let mut navmesh_path: Option<PathBuf> = None;
    let mut bot_population_path: Option<PathBuf> = None;
    let mut bot_names_path: Option<PathBuf> = None;
    let mut bot_fill_total: usize = 0;
    let mut initial_spawn_delay_ms: Option<[u32; 2]> = None;
    let mut match_profiles_path: Option<PathBuf> = None;
    let mut spawn_path: Option<PathBuf> = None;
    let mut ground_vehicles_path: Option<PathBuf> = None;
    let mut dynamic_bodies_path: Option<PathBuf> = None;
    let mut objective_path: Option<PathBuf> = None;
    let mut item_path: Option<PathBuf> = None;
    let mut blast_rules_path: Option<PathBuf> = None;
    let mut retail_metadata_path: Option<PathBuf> = None;
    let mut bootstrap_loadouts_path: Option<PathBuf> = None;
    let mut fall_rules_path: Option<PathBuf> = None;
    let mut incapacitation_rules_path: Option<PathBuf> = None;
    let mut character_damage_rules_path: Option<PathBuf> = None;
    let mut skill_path: Option<PathBuf> = None;
    let mut retail_armory_path: Option<PathBuf> = None;
    let mut leadership_passives_path: Option<PathBuf> = None;
    let mut leadership_role_policy_path: Option<PathBuf> = None;
    let mut structure_path: Option<PathBuf> = None;
    let mut bunker_path: Option<PathBuf> = None;
    let mut bunker_exit_policy_path: Option<PathBuf> = None;
    let mut wire_profiles_path: Option<PathBuf> = None;
    let mut collision_query = None;
    let mut development_dummy = false;
    let mut development_dummy_position = None;
    let mut development_control = false;
    let mut development_grenade_radius = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--match-profiles" => {
                match_profiles_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--match-profiles requires an absolute manifest path")
                        })?
                        .into(),
                );
            }
            "--application-id" => {
                application_id = Some(parse_application_id(&args.next().ok_or_else(|| {
                    io::Error::other("--application-id requires 20230 or 20231")
                })?)?);
            }
            "--validate-catalogs" => validate_catalogs = true,
            "--headless-sim-seconds" => {
                headless_sim_seconds = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--headless-sim-seconds requires N"))?
                        .parse::<u64>()
                        .map_err(|_| io::Error::other("invalid --headless-sim-seconds"))?,
                );
            }
            "--bot-names" => {
                bot_names_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--bot-names requires a path"))?
                        .into(),
                );
            }
            "--bot-population" => {
                bot_population_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--bot-population requires a path"))?
                        .into(),
                );
            }
            "--initial-spawn-delay-ms" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::other("--initial-spawn-delay-ms requires milliseconds")
                })?;
                initial_spawn_delay_ms = Some(parse_team_delays(&value).ok_or_else(|| {
                    io::Error::other("invalid --initial-spawn-delay-ms (use N or TEAM0,TEAM1)")
                })?);
            }
            "--bot-fill-total" => {
                bot_fill_total = args
                    .next()
                    .ok_or_else(|| io::Error::other("--bot-fill-total requires a count"))?
                    .parse()
                    .map_err(|_| io::Error::other("invalid --bot-fill-total"))?;
            }
            "--navmesh-world" => {
                navmesh_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--navmesh-world requires a path"))?
                        .into(),
                );
            }
            "--retail-combat-metadata" => {
                retail_metadata_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--retail-combat-metadata requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--character-blast-rules" => {
                blast_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--character-blast-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--wire-profiles" => {
                wire_profiles_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--wire-profiles requires a path"))?
                        .into(),
                );
            }
            "--tick-telemetry" => {
                telemetry_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--tick-telemetry requires a NEW path"))?
                        .into(),
                );
            }
            "--tick-budget-ms" => {
                let value: f64 = args
                    .next()
                    .ok_or_else(|| io::Error::other("--tick-budget-ms requires a number"))?
                    .parse()
                    .map_err(|_| io::Error::other("invalid --tick-budget-ms"))?;
                if !value.is_finite() || !(0.001..=1000.0).contains(&value) {
                    return Err(io::Error::other("tick budget must be .001..1000 ms"));
                }
                tick_budget_us = Some((value * 1000.0).round() as u64);
            }
            "--character-damage-rules" => {
                character_damage_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--character-damage-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--incapacitation-rules" => {
                incapacitation_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--incapacitation-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--fall-damage-rules" => {
                fall_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--fall-damage-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--leadership-passives" => {
                leadership_passives_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--leadership-passives requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--leadership-role-policy" => {
                leadership_role_policy_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--leadership-role-policy requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--retail-armory-catalog" => {
                retail_armory_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--retail-armory-catalog requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--skill-catalog" => {
                skill_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--skill-catalog requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--structure-catalog" => {
                structure_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--structure-catalog requires a path"))?
                        .into(),
                );
            }
            "--bunker-exit-policy" => {
                bunker_exit_policy_path =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        io::Error::other("--bunker-exit-policy requires a path")
                    })?));
            }
            "--bunker-catalog" => {
                bunker_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--bunker-catalog requires a path"))?
                        .into(),
                );
            }
            "--bootstrap-loadouts" => {
                bootstrap_loadouts_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--bootstrap-loadouts requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--item-catalog" => {
                item_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--item-catalog requires a path"))?
                        .into(),
                );
            }
            "--objective-catalog" => {
                objective_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--objective-catalog requires a path"))?
                        .into(),
                );
            }
            "--dynamic-bodies" => {
                dynamic_bodies_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--dynamic-bodies requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--ground-vehicles" => {
                ground_vehicles_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--ground-vehicles requires a path"))?
                        .into(),
                );
            }
            "--spawn-catalog" => {
                spawn_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--spawn-catalog requires a path"))?
                        .into(),
                );
            }
            "--collision-world" => {
                collision_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--collision-world requires a path"))?
                        .into(),
                );
            }
            "--collision-query" => {
                let start = args.next().ok_or_else(|| {
                    io::Error::other("--collision-query requires START END as x,y,z")
                })?;
                let end = args.next().ok_or_else(|| {
                    io::Error::other("--collision-query requires START END as x,y,z")
                })?;
                collision_query = Some((parse_point(&start)?, parse_point(&end)?));
            }
            "--development-dummy-position" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::other("--development-dummy-position requires x,y,z")
                })?;
                let [x, y, z] = parse_point(&value)?;
                let position = mag_gameplay::replication::CharacterPosition {
                    x: x as f32,
                    y: y as f32,
                    z: z as f32,
                };
                development_dummy_position = Some(position);
            }
            "--development-dummy" => development_dummy = true,
            "--development-control" => development_control = true,
            "--development-grenade-radius" => development_grenade_radius = true,
            "--bind" => {
                bind = args
                    .next()
                    .ok_or_else(|| io::Error::other("--bind requires an address"))?
                    .parse()
                    .map_err(|_| io::Error::other("invalid --bind address"))?
            }
            "--log" => {
                log = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--log requires a new path"))?
                        .into(),
                )
            }
            "--log-segment-bytes" => {
                log_segment_bytes = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--log-segment-bytes requires a byte count")
                        })?
                        .parse()
                        .map_err(|_| io::Error::other("invalid --log-segment-bytes"))?,
                );
            }
            "--log-retention-s" => {
                log_retention_s = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--log-retention-s requires seconds"))?
                        .parse()
                        .map_err(|_| io::Error::other("invalid --log-retention-s"))?,
                );
            }
            _ => {
                return Err(io::Error::other(
                    "usage: mag-gameplay [--application-id 20230|20231] [--validate-catalogs] --item-catalog ITEMS.json --spawn-catalog SPAWNS.json --log NEW_FILE [--match-profiles /absolute/MATCH_PROFILES.json] [--wire-profiles PROFILES.json] [--objective-catalog OBJECTIVES.json] [--ground-vehicles /absolute/VEHICLES.json] [--dynamic-bodies /absolute/BODIES.json] [--character-blast-rules /absolute/BLAST.json] [--retail-combat-metadata /absolute/RETAIL.json] [--bootstrap-loadouts /absolute/BOOTSTRAP.json] [--fall-damage-rules /absolute/FALL.json] [--incapacitation-rules /absolute/INCAPACITATION.json] [--character-damage-rules /absolute/DAMAGE.json] [--skill-catalog /absolute/SKILLS.json] [--retail-armory-catalog /absolute/ARMORY.json] [--leadership-passives /absolute/PASSIVES.json --leadership-role-policy /absolute/ROLES.json] [--bind IP:PORT] [--development-control] [--development-grenade-radius] [--development-dummy] [--development-dummy-position x,y,z] [--collision-world WORLD.json] [--navmesh-world MESH.mag-nav] [--bot-fill-total N --bot-population POPULATION.json] [--bot-names /absolute/NAMES.json] [--tick-telemetry NEW_FILE] [--tick-budget-ms MS] [--log-segment-bytes BYTES [--log-retention-s SECONDS]]; query only: mag-gameplay --collision-world WORLD.json --collision-query x,y,z x,y,z",
                ));
            }
        }
    }
    if match_profiles_path.is_some() {
        if application_id != Some(20231) {
            return Err(io::Error::other(
                "--match-profiles requires --application-id 20231",
            ));
        }
        if bot_population_path.is_some()
            || bot_fill_total != 0
            || development_dummy
            || structure_path.is_some()
            || bunker_path.is_some()
            || bunker_exit_policy_path.is_some()
            || leadership_passives_path.is_some()
            || leadership_role_policy_path.is_some()
            || collision_query.is_some()
        {
            return Err(io::Error::other(
                "--match-profiles does not support bots, bunkers, leadership, dummy or collision-query",
            ));
        }
    }
    if validate_catalogs && collision_query.is_some() {
        return Err(io::Error::other(
            "--validate-catalogs cannot be combined with --collision-query",
        ));
    }
    if development_dummy_position.is_some() && !development_dummy {
        return Err(io::Error::other(
            "--development-dummy-position requires --development-dummy",
        ));
    }
    // Geometry is loaded before opening logs/listeners. A bad file cannot
    // silently fall back to an empty collision world or accept a reservation.
    let collision_world = collision_path
        .as_ref()
        .map(|path| {
            CollisionWorld::load(path, CollisionLimits::default()).map_err(io::Error::other)
        })
        .transpose()?;
    let navmesh_world = if let Some(path) = navmesh_path {
        let world = collision_world
            .as_ref()
            .ok_or_else(|| io::Error::other("--navmesh-world requires --collision-world"))?;
        if std::fs::metadata(&path)?.len() > mag_gameplay::navmesh::MAX_ARTIFACT_JSON_BYTES {
            return Err(io::Error::other("navmesh artifact size limit"));
        }
        let artifact: mag_gameplay::navmesh::Artifact =
            serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)?;
        let hash = mag_gameplay::navmesh::sha256(&std::fs::read(collision_path.as_ref().unwrap())?);
        let mesh =
            mag_gameplay::navmesh::NavMesh::load(artifact, &hash, &world.metadata().map_name)
                .map_err(io::Error::other)?;
        mesh.validate_coverage(world).map_err(io::Error::other)?;
        Some(mesh)
    } else {
        None
    };
    if let Some((start, end)) = collision_query {
        if log.is_some()
            || telemetry_path.is_some()
            || tick_budget_us.is_some()
            || log_segment_bytes.is_some()
            || log_retention_s.is_some()
        {
            return Err(io::Error::other(
                "--collision-query is standalone; omit --log and telemetry flags",
            ));
        }
        let world = collision_world
            .as_ref()
            .ok_or_else(|| io::Error::other("--collision-query requires --collision-world"))?;
        let query = world
            .raycast_segment(start, end)
            .map_err(io::Error::other)?;
        println!(
            "{}",
            serde_json::json!({"event":"collision_query", "source":collision_path,
            "stats":world.stats(), "query":query, "authority":"diagnostic_only"})
        );
        return Ok(());
    }
    if log_retention_s.is_some() && log_segment_bytes.is_none() {
        return Err(io::Error::other(
            "--log-retention-s requires --log-segment-bytes",
        ));
    }
    let rotation = log_segment_bytes
        .map(|segment_bytes| {
            mag_gameplay::rotating_log::RotationPolicy {
                segment_bytes,
                retention: log_retention_s.map(Duration::from_secs),
            }
            .validate()
            .map_err(io::Error::other)
        })
        .transpose()?;
    if tick_budget_us.is_some() && telemetry_path.is_none() {
        return Err(io::Error::other(
            "--tick-budget-ms requires --tick-telemetry",
        ));
    }
    if validate_catalogs && application_id.is_none() {
        return Err(io::Error::other(
            "--validate-catalogs requires --application-id",
        ));
    }
    validate_runtime_paths(
        application_id,
        &[
            ("--item-catalog", item_path.as_deref()),
            ("--spawn-catalog", spawn_path.as_deref()),
            ("--wire-profiles", wire_profiles_path.as_deref()),
            ("--character-blast-rules", blast_rules_path.as_deref()),
            ("--bootstrap-loadouts", bootstrap_loadouts_path.as_deref()),
            ("--retail-combat-metadata", retail_metadata_path.as_deref()),
            ("--retail-armory-catalog", retail_armory_path.as_deref()),
            ("--skill-catalog", skill_path.as_deref()),
            ("--fall-damage-rules", fall_rules_path.as_deref()),
        ],
    )?;
    if application_id == Some(20231) {
        // Required paths were checked above. Verify projections identify these exact
        // item bytes rather than accepting unrelated catalogs with the same app ID.
        validate_item_catalog_links(
            item_path.as_deref().unwrap(),
            &[
                (
                    "bootstrap loadouts",
                    bootstrap_loadouts_path.as_deref().unwrap(),
                    "/provenance/catalog_sha256",
                ),
                (
                    "retail combat",
                    retail_metadata_path.as_deref().unwrap(),
                    "/provenance/catalog_sha256",
                ),
                (
                    "retail Armory",
                    retail_armory_path.as_deref().unwrap(),
                    "/source_sha256/gameplay~1data~1retail-items.json",
                ),
                (
                    "retail skills",
                    skill_path.as_deref().unwrap(),
                    "/source_sha256/gameplay~1data~1retail-items.json",
                ),
            ],
        )?;
    }
    if application_id == Some(20231)
        && (structure_path.is_some() || bunker_path.is_some() || bunker_exit_policy_path.is_some())
    {
        return Err(io::Error::other(
            "retail structure/bunker authority is unverified; omit structure/bunker inputs for the supported no-bunker profile",
        ));
    }
    // Validate once before listening; the installed catalog resolves ordinary retail hits.
    let mut retail_combat_catalog = None;
    let retail_metadata = match &retail_metadata_path {
        Some(path) => {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "retail combat metadata requires an absolute path",
                ));
            }
            let data = mag_gameplay::retail_combat::RetailCombatCatalog::load(path)
                .map_err(|error| io::Error::other(error.to_string()))?;
            let info = Some(
                serde_json::json!({"path":path,"application_id":data.application_id(),"scope":"ordinary_client_reported_bullets",
                "bullets":data.bullets().len(),"grenades":data.grenades().len(),"armor":data.armor().len(),"body":data.body().len(),
                "healing":data.healing().len(),"authority":data.authority()}),
            );
            retail_combat_catalog = Some(data);
            info
        }
        None => None,
    };
    let spawn_path = spawn_path
        .ok_or_else(|| io::Error::other("--spawn-catalog is required for gameplay startup"))?;
    let catalog = mag_gameplay::spawn_catalog::SpawnCatalog::load(&spawn_path)?;
    let catalog = match initial_spawn_delay_ms {
        Some(delay) => catalog
            .with_initial_spawn_delay_ms(delay)
            .map_err(io::Error::other)?,
        None => catalog,
    };
    let effective_initial_spawn_delay_ms = catalog.match_profile().initial_spawn_delay_ms;
    if ground_vehicles_path
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err(io::Error::other(
            "--ground-vehicles requires an explicit absolute path",
        ));
    }
    if ground_vehicles_path.is_some() && application_id != Some(20231) {
        return Err(io::Error::other(
            "--ground-vehicles requires explicit retail application20231",
        ));
    }
    let ground_vehicles = ground_vehicles_path
        .as_ref()
        .map(|path| mag_gameplay::ground_vehicles::Catalog::load(path, &catalog))
        .transpose()?;
    let dynamic_bodies = dynamic_bodies_path
        .as_ref()
        .map(|path| -> io::Result<_> {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--dynamic-bodies requires an absolute path",
                ));
            }
            if application_id != Some(20231) || catalog.match_profile().game_type != 1 {
                return Err(io::Error::other(
                    "dynamic bodies require retail Acquisition",
                ));
            }
            let bodies = mag_gameplay::dynamic_collision::Catalog::load(path)?;
            let bindings = bodies
                .runtime_bindings()
                .cloned()
                .ok_or_else(|| io::Error::other("dynamic_body_bindings_required"))?;
            bindings
                .validate_for(&bodies, ground_vehicles.as_ref())
                .map_err(io::Error::other)?;
            Ok((bodies, bindings))
        })
        .transpose()?;
    if application_id.is_some()
        && collision_world
            .as_ref()
            .is_some_and(|world| world.metadata().map_name != catalog.map_name())
    {
        return Err(io::Error::other(
            "collision world map does not match the selected spawn catalog",
        ));
    }
    if application_id == Some(20231) && catalog.match_profile().has_bunkers {
        return Err(io::Error::other(
            "retail bunker features are unsupported without verified retail authority",
        ));
    }
    let collision_info = collision_world.as_ref().map(|world| serde_json::json!({
        "source":collision_path,"metadata":world.metadata(),"stats":world.stats(),"bullet_authority":"audit_only","grenade_policy":"disk_triangles_three_body_samples","grenade_radius_enabled":development_grenade_radius,"active_map_name":catalog.map_name(),"map_matches_active":world.metadata().map_name == catalog.map_name()}));
    let objective_catalog = objective_path
        .as_ref()
        .map(mag_gameplay::objective_catalog::ObjectiveCatalog::load)
        .transpose()?;
    if let Some(objectives) = &objective_catalog {
        objectives
            .validate_match(catalog.map_name(), catalog.match_profile())
            .map_err(io::Error::other)?;
    }
    catalog
        .validate_objectives(objective_catalog.as_ref())
        .map_err(io::Error::other)?;
    let objective_info = objective_catalog.as_ref().map(|objectives| serde_json::json!({"path":objective_path,"object_count":objectives.object_count(),"source":objectives.source()}));
    let item_path = item_path
        .ok_or_else(|| io::Error::other("--item-catalog is required for gameplay startup"))?;
    let blast_rules_path =
        blast_rules_path.unwrap_or_else(|| item_path.with_file_name("character-blast-rules.json"));
    if !blast_rules_path.is_absolute() {
        return Err(io::Error::other(
            "character-blast rules require an absolute path",
        ));
    }
    let mut item_catalog = match application_id {
        Some(id) => mag_gameplay::item_catalog::ItemCatalog::load_for_application(
            &item_path,
            &blast_rules_path,
            id,
        )?,
        None => mag_gameplay::item_catalog::ItemCatalog::load_with_blast_rules(
            &item_path,
            &blast_rules_path,
        )?,
    };
    if let Some(path) = &bootstrap_loadouts_path {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "--bootstrap-loadouts requires an absolute path",
            ));
        }
        item_catalog = item_catalog.with_bootstrap_loadouts(
            mag_gameplay::bootstrap_loadouts::BootstrapLoadouts::load(path)?,
        );
    }
    item_catalog
        .validate_bootstrap_faction(catalog.match_profile().defending_faction)
        .map_err(io::Error::other)?;
    if let Some(combat) = retail_combat_catalog {
        item_catalog = item_catalog.with_retail_combat(combat);
    }
    if let Some(path) = &retail_armory_path {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "--retail-armory-catalog requires an absolute path",
            ));
        }
        item_catalog =
            item_catalog.with_retail_armory(mag_gameplay::retail_armory::RetailArmory::load(path)?);
    }
    if application_id == Some(20231) {
        let armory = item_catalog
            .retail_armory()
            .ok_or_else(|| io::Error::other("retail Armory required"))?;
        for faction in 0..3 {
            let bootstrap = item_catalog.bootstrap_loadout(faction, 0).ok_or_else(|| {
                io::Error::other("retail bootstrap must include all supported factions")
            })?;
            let saved = mag_gameplay::loadouts::SavedLoadouts {
                selected_index: 0,
                update_ids: std::array::from_fn(|index| bootstrap.inventories[index].update_id),
                inventories: std::array::from_fn(|index| bootstrap.inventories[index].slots),
            };
            for inventory in &saved.inventories {
                for &item in inventory.iter().flatten() {
                    item_catalog.validate_item(item).map_err(io::Error::other)?;
                }
            }
            armory
                .validate_saved(faction, &saved)
                .map_err(io::Error::other)?;
        }
    }
    let item_catalog = Arc::new(item_catalog);
    let wire_profiles_path =
        wire_profiles_path.unwrap_or_else(|| item_path.with_file_name("wire-profiles.json"));
    let wire_profiles = mag_gameplay::wire_profile::WireProfileCatalog::load(&wire_profiles_path)?;
    if let Some(id) = application_id {
        let profile = wire_profiles.profile(id).map_err(io::Error::other)?;
        if let Some(objectives) = &objective_catalog {
            objectives
                .validate_profile(profile)
                .map_err(io::Error::other)?;
        } else if catalog.match_profile().has_objectives {
            return Err(io::Error::other(
                "objective catalog required when the match profile enables objectives",
            ));
        }
    }
    if skill_path.as_ref().is_some_and(|path| !path.is_absolute()) {
        return Err(io::Error::other(
            "--skill-catalog requires an absolute path",
        ));
    }
    let skill_path = match skill_path {
        Some(path) => path,
        None => std::env::current_dir()?
            .join(
                item_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(".")),
            )
            .join("prototype-skills.json"),
    };
    let skill_catalog = mag_gameplay::skill_catalog::SkillCatalog::load(&skill_path)?;
    if application_id.is_some_and(|id| skill_catalog.application_id() != id) {
        return Err(io::Error::other(
            "skill catalog application does not match --application-id",
        ));
    }
    let leadership = match (&leadership_passives_path, &leadership_role_policy_path) {
        (None, None) => None,
        (Some(passives), Some(policy)) => {
            if !passives.is_absolute() || !policy.is_absolute() {
                return Err(io::Error::other(
                    "leadership configuration paths must be absolute",
                ));
            }
            let catalog = mag_gameplay::leadership_passives::LeadershipCatalog::load(passives)?;
            let policy = mag_gameplay::leadership_runtime::RolePolicy::load(policy)?;
            if policy.has_development_bindings() && !development_control {
                return Err(io::Error::other(
                    "leadership development bindings require --development-control",
                ));
            }
            Some((catalog, policy))
        }
        _ => {
            return Err(io::Error::other(
                "--leadership-passives and --leadership-role-policy must be supplied together",
            ));
        }
    };
    let structure_path =
        structure_path.unwrap_or_else(|| item_path.with_file_name("prototype-structures.json"));
    let bunker_path =
        bunker_path.unwrap_or_else(|| item_path.with_file_name("prototype-bunkers.json"));
    let bunker_world = if application_id == Some(20231) {
        None
    } else if catalog.match_profile().has_bunkers {
        Some(mag_gameplay::structures::BunkerWorld::load(
            &bunker_path,
            &structure_path,
            catalog.map_name(),
        )?)
    } else {
        Some(mag_gameplay::structures::BunkerWorld::without_bunkers(
            &structure_path,
        )?)
    };
    let bunker_exit_policy = bunker_exit_policy_path
        .as_ref()
        .map(mag_gameplay::vehicle_exit::BunkerExitPolicy::load)
        .transpose()?;
    let bunker_info = match &bunker_world {
        Some(world) => {
            serde_json::json!({"active":world.active_count(),"unresolved":world.unresolved_count(),"placement_source":bunker_path,"combat_source":structure_path})
        }
        None => {
            serde_json::json!({"enabled":false,"reason":"retail_authority_unverified","placement_source":null,"combat_source":null})
        }
    };
    if let Some(position) = development_dummy_position {
        mag_gameplay::dummy::respawn_at_with_catalog_and_profile(
            2,
            0,
            position,
            &item_catalog,
            wire_profiles
                .profile(application_id.unwrap_or(20230))
                .map_err(io::Error::other)?,
        )
        .map_err(io::Error::other)?;
    }
    let character_damage_rules = character_damage_rules_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--character-damage-rules requires an absolute path",
                ));
            }
            if application_id != Some(20231) {
                return Err(io::Error::other(
                    "character damage rules require the retail application profile",
                ));
            }
            let rules = mag_gameplay::character_damage_rules::Rules::load(path)?;
            rules
                .validate_match(20231, &catalog)
                .map_err(io::Error::other)?;
            Ok(rules)
        })
        .transpose()?;
    let incapacitation_rules = incapacitation_rules_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--incapacitation-rules requires an absolute path",
                ));
            }
            mag_gameplay::incapacitation::Rules::load(path)
        })
        .transpose()?;
    if incapacitation_rules.is_some() {
        if application_id != Some(20231) {
            return Err(io::Error::other(
                "incapacitation requires the retail application profile",
            ));
        }
        let combat = item_catalog.retail_combat().map_err(io::Error::other)?;
        for location in 0..16 {
            let value = combat
                .body_for_damage(location)
                .map_err(io::Error::other)?
                .incapacitated_multiplier
                .ok_or_else(|| {
                    io::Error::other("incapacitation requires explicit body multipliers")
                })?;
            if !(value as f32).is_finite() {
                return Err(io::Error::other(
                    "incapacitation body multiplier is not representable",
                ));
            }
        }
    }
    let fall_rules = fall_rules_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--fall-damage-rules requires an absolute path",
                ));
            }
            mag_gameplay::fall_damage::Rules::load(path)
        })
        .transpose()?;
    let bot_names = bot_names_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other("--bot-names requires an absolute path"));
            }
            mag_gameplay::bot_names::Catalog::load(path)
                .map_err(|e| io::Error::other(format!("failed to load bot names: {e}")))
        })
        .transpose()?
        .unwrap_or_default();
    let bot_names_count = bot_names.len();
    if bot_fill_total > bot_names_count {
        return Err(io::Error::other(
            "bot names have insufficient capacity for --bot-fill-total",
        ));
    }
    let actual_dummy_position = if development_dummy {
        Some(
            development_dummy_position
                .or_else(|| catalog.alternate_position())
                .ok_or_else(|| io::Error::other("missing_alternate_spawn"))?,
        )
    } else {
        None
    };
    let match_profiles = match_profiles_path
        .as_ref()
        .map(mag_gameplay::match_profiles::Registry::load)
        .transpose()?;
    if let Some(registry) = &match_profiles {
        registry
            .validate_runtime(
                &item_catalog,
                wire_profiles.profile(20231).map_err(io::Error::other)?,
            )
            .map_err(io::Error::other)?;
        if let Some((catalog, bindings)) = &dynamic_bodies {
            registry
                .validate_dynamic_bodies(catalog, bindings)
                .map_err(io::Error::other)?;
        }
    }
    let mut resources = match application_id {
        Some(id) => {
            EngineResources::for_application(item_catalog, catalog, bunker_world, wire_profiles, id)
                .map_err(io::Error::other)?
        }
        None => EngineResources::new(
            item_catalog,
            catalog,
            bunker_world.ok_or_else(|| io::Error::other("legacy bunker resources missing"))?,
        )
        .with_wire_profiles(wire_profiles),
    };
    if let Some(policy) = bunker_exit_policy {
        resources = resources
            .with_bunker_exit_policy(policy)
            .map_err(io::Error::other)?;
    }
    resources = resources.with_bot_names(bot_names);
    if validate_catalogs {
        if log.is_some() || telemetry_path.is_some() || rotation.is_some() {
            return Err(io::Error::other(
                "--validate-catalogs does not open logs; omit --log, --tick-telemetry and log rotation flags",
            ));
        }
        println!(
            "{}",
            serde_json::json!({"event":"catalogs_validated","application_id":application_id,"skill_application_id":skill_catalog.application_id(),"bunkers":bunker_info,"fall_damage_enabled":fall_rules.is_some(),"incapacitation_enabled":incapacitation_rules.is_some(),"character_damage_rules_enabled":character_damage_rules.is_some(),"dynamic_body_catalog_validated":dynamic_bodies.is_some(),"scope":"schema_and_profile_validation_not_client_acceptance"})
        );
        return Ok(());
    }
    let path = log.ok_or_else(|| {
        io::Error::other("--log is required; existing logs are never overwritten")
    })?;
    // Rotation removes the whole-file cap; retention, not a byte ceiling, then bounds disk use.
    let max_log = if rotation.is_some() {
        u64::MAX
    } else {
        MAX_LOG
    };
    let telemetry = telemetry_path
        .as_ref()
        .map(|path| open_log(path, rotation).map(|writer| TickRecorder::start(writer, max_log)))
        .transpose()?;
    let writer = open_log(&path, rotation)?;
    let mut engine =
        Engine::from_resources(writer, max_log, resources).with_skill_catalog(skill_catalog);
    if let Some(vehicles) = ground_vehicles {
        engine = engine
            .with_ground_vehicles(vehicles)
            .map_err(io::Error::other)?;
    }
    if let Some((catalog, bindings)) = dynamic_bodies {
        engine = engine
            .with_dynamic_collision_bodies(catalog, bindings)
            .map_err(io::Error::other)?;
    }
    if let Some(objectives) = objective_catalog {
        engine = engine
            .with_objective_catalog(objectives)
            .map_err(io::Error::other)?;
    }
    if let Some(rules) = character_damage_rules {
        engine = engine
            .with_character_damage_rules(rules)
            .map_err(io::Error::other)?;
    }
    if let Some(rules) = incapacitation_rules {
        engine = engine
            .with_incapacitation_rules(rules)
            .map_err(io::Error::other)?;
    }
    if let Some(rules) = fall_rules {
        engine = engine.with_fall_damage_rules(rules);
    }
    if let Some(world) = collision_world {
        engine = engine.with_collision_world(world);
    }
    if let Some(mesh) = navmesh_world {
        engine = engine.with_navmesh_world(mesh);
    }
    if development_control {
        engine = engine.with_development_control();
    }
    if development_grenade_radius {
        engine = engine.with_development_grenade_radius();
    }
    if development_dummy {
        engine = engine.with_development_dummy();
    }
    if let Some(position) = development_dummy_position {
        engine = engine
            .with_development_dummy_position(position)
            .map_err(io::Error::other)?;
    }
    if let Some((catalog, policy)) = leadership {
        engine = engine
            .with_leadership_passives(catalog, policy)
            .map_err(io::Error::other)?;
    }
    if bot_fill_total > 0 {
        let path = bot_population_path
            .as_ref()
            .ok_or_else(|| io::Error::other("--bot-fill-total requires --bot-population"))?;
        let population = mag_gameplay::bot_registry::BotPopulation::load(path)
            .map_err(|e| io::Error::other(e.to_string()))?;
        engine = engine
            .with_bot_population(population, bot_fill_total)
            .map_err(io::Error::other)?;
    }
    if let Some(registry) = match_profiles {
        engine = engine
            .with_match_profiles(registry)
            .map_err(io::Error::other)?;
    }
    if let Some(seconds) = headless_sim_seconds {
        // LOCAL DIAGNOSTIC: virtual-time match with one never-spawning observer.
        let faction: u8 = std::env::var("MAG_SIM_FACTION")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let wall0 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let began = Instant::now();
        engine.enable_world_ticks();
        engine
            .headless_sim_join(Duration::ZERO, wall0, faction)
            .map_err(io::Error::other)?;
        let step = Duration::from_millis(50);
        let mut now = step;
        let mut errors = 0u64;
        while now <= Duration::from_secs(seconds) {
            engine.headless_sim_keepalive(now);
            let r = engine.tick_world(now, wall0 + now.as_secs(), "headless-sim".into());
            if !r.ok {
                errors += 1;
                if errors <= 5 {
                    eprintln!("tick {} error {:?}", now.as_millis(), r.error);
                }
            }
            now += step;
        }
        eprintln!(
            "{}",
            serde_json::json!({"event":"headless_sim_done","simulated_s":seconds,"wall_ms":began.elapsed().as_millis(),"tick_errors":errors})
        );
        return Ok(());
    }
    let engine = Arc::new(Mutex::new(engine));
    let listener = TcpListener::bind(bind)?;
    let started = Instant::now();
    let active = Arc::new(AtomicUsize::new(0));
    let subscriber: SharedSubscriber = Arc::new(Mutex::new(None));
    spawn_world_scheduler(
        Arc::clone(&engine),
        Arc::clone(&subscriber),
        started,
        telemetry.clone(),
        tick_budget_us,
    );
    println!(
        "{}",
        serde_json::json!({"event":"listening","application_id":application_id.unwrap_or(20230),"match_profiles":match_profiles_path,"profile_selection":if application_id.is_some(){"explicit"}else{"legacy"},"protocol":mag_gameplay::BRIDGE_VERSION,"address":listener.local_addr()?.to_string(),"collision":collision_info,"spawn_catalog":spawn_path,"objective_catalog":objective_info,"item_catalog":item_path,"character_blast_rules":blast_rules_path,"retail_combat_metadata":retail_metadata,"bootstrap_loadouts":bootstrap_loadouts_path,"fall_damage_rules":fall_rules_path,"incapacitation_rules":incapacitation_rules_path,"character_damage_rules":character_damage_rules_path,"skill_catalog":skill_path,"retail_armory_catalog":retail_armory_path,"leadership_passives":leadership_passives_path,"leadership_role_policy":leadership_role_policy_path,"bunkers":bunker_info,"tick_telemetry":telemetry_path,"tick_budget_us":tick_budget_us,"scheduler":"independent_50ms_when_subscribed","bot_fill_total":bot_fill_total,"initial_spawn_delay_ms":effective_initial_spawn_delay_ms,"initial_spawn_delay_source":if initial_spawn_delay_ms.is_some(){"override"}else{"catalog"},"bot_population":bot_population_path,"bot_names":bot_names_path,"bot_names_count":bot_names_count,"development_dummy_position":actual_dummy_position})
    );
    for connection in listener.incoming() {
        let stream = connection?;
        if active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 8).then_some(n + 1)
            })
            .is_err()
        {
            drop(stream);
            continue;
        }
        let engine = Arc::clone(&engine);
        let active = Arc::clone(&active);
        let telemetry = telemetry.clone();
        let subscriber = Arc::clone(&subscriber);
        std::thread::spawn(move || {
            let slot = ConnectionSlot(Some(active));
            if let Err(failure) = serve(
                stream,
                engine,
                started,
                telemetry,
                tick_budget_us,
                subscriber,
                slot,
            ) {
                // Fixed categories only: no payload, request ID, credentials or
                // operating-system error text can reach this diagnostic.
                let _ = writeln!(io::stderr().lock(), "{}", failure.diagnostic());
            }
        });
    }
    Ok(())
}
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ConnectionStage {
    ReadRequest,
    EncodeResponse,
    WriteResponse,
}

struct ConnectionFailure {
    stage: ConnectionStage,
    kind: io::ErrorKind,
}
impl ConnectionFailure {
    fn io(stage: ConnectionStage, error: io::Error) -> Self {
        Self {
            stage,
            kind: error.kind(),
        }
    }
    fn diagnostic(&self) -> serde_json::Value {
        let category = match (self.stage, self.kind) {
            (ConnectionStage::EncodeResponse, _) => "encoding_failed",
            (_, io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => "timeout",
            (_, io::ErrorKind::InvalidData) => "invalid_framing",
            (_, io::ErrorKind::UnexpectedEof) => "incomplete_request",
            (
                _,
                io::ErrorKind::BrokenPipe
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::NotConnected,
            ) => "disconnected",
            _ => "io_error",
        };
        serde_json::json!({"event":"connection_error", "stage":self.stage, "category":category})
    }
}

fn serve(
    mut stream: TcpStream,
    engine: Arc<Mutex<Engine<LogWriter>>>,
    started: Instant,
    telemetry: Option<TickRecorder>,
    tick_budget_us: Option<u64>,
    subscriber: SharedSubscriber,
    slot: ConnectionSlot,
) -> Result<(), ConnectionFailure> {
    let line = read_line(&mut stream, IO_DEADLINE)
        .map_err(|error| ConnectionFailure::io(ConnectionStage::ReadRequest, error))?;
    if serde_json::from_slice::<serde_json::Value>(&line)
        .ok()
        .is_some_and(|v| v["op"] == "subscribe_world")
    {
        return serve_subscription(stream, &line, engine, subscriber, slot);
    }
    let response = match parse_request(&line) {
        Ok(request) => {
            let wall = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let update = telemetry
                .as_ref()
                .is_some_and(|_| request.is_gameplay_update());
            let waiting = Instant::now();
            let started_us = started.elapsed().as_micros() as u64;
            let (response, wait_us, engine_us) = match engine.lock() {
                Ok(mut engine) => {
                    let wait_us = waiting.elapsed().as_micros() as u64;
                    let processing = Instant::now();
                    let response = engine.handle(request, started.elapsed(), wall);
                    (response, wait_us, processing.elapsed().as_micros() as u64)
                }
                Err(_) => (
                    Response::error(String::new(), "engine_unavailable"),
                    waiting.elapsed().as_micros() as u64,
                    0,
                ),
            };
            // The engine lock has been released before telemetry enqueue.
            if let Some(recorder) = telemetry.as_ref() {
                recorder.record(TickSample::new(
                    update,
                    started_us,
                    wait_us,
                    engine_us,
                    response.ok,
                    tick_budget_us,
                ));
            }
            response
        }
        Err(error) => Response::error(error.request_id, error.code),
    };
    let mut reply = serde_json::to_vec(&response).map_err(|_| ConnectionFailure {
        stage: ConnectionStage::EncodeResponse,
        kind: io::ErrorKind::InvalidData,
    })?;
    reply.push(b'\n');
    stream
        .set_write_timeout(Some(IO_DEADLINE))
        .and_then(|_| stream.write_all(&reply))
        .map_err(|error| ConnectionFailure::io(ConnectionStage::WriteResponse, error))
}

fn parse_point(value: &str) -> io::Result<[f64; 3]> {
    let parts: Vec<_> = value.split(',').collect();
    if parts.len() != 3 {
        return Err(io::Error::other("point requires x,y,z"));
    }
    let mut result = [0.0_f64; 3];
    for (out, part) in result.iter_mut().zip(parts) {
        *out = part
            .parse()
            .map_err(|_| io::Error::other("invalid point coordinate"))?;
        if !out.is_finite() {
            return Err(io::Error::other("point must be finite"));
        }
    }
    Ok(result)
}

struct ConnectionSlot(Option<Arc<AtomicUsize>>);
impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        if let Some(active) = self.0.take() {
            active.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
struct WorldSubscriber {
    generation: u64,
    world_id: u32,
    request_id: String,
    sender: SyncSender<Response>,
}
type SharedSubscriber = Arc<Mutex<Option<WorldSubscriber>>>;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscribeWorld {
    op: String,
    v: u8,
    request_id: String,
    world_id: u32,
}
fn write_response(stream: &mut TcpStream, response: &Response) -> Result<(), ConnectionFailure> {
    let mut bytes = serde_json::to_vec(response).map_err(|_| ConnectionFailure {
        stage: ConnectionStage::EncodeResponse,
        kind: io::ErrorKind::InvalidData,
    })?;
    bytes.push(b'\n');
    stream
        .set_write_timeout(Some(IO_DEADLINE))
        .and_then(|_| stream.write_all(&bytes))
        .map_err(|e| ConnectionFailure::io(ConnectionStage::WriteResponse, e))
}
fn serve_subscription(
    mut stream: TcpStream,
    line: &[u8],
    engine: Arc<Mutex<Engine<LogWriter>>>,
    shared: SharedSubscriber,
    slot: ConnectionSlot,
) -> Result<(), ConnectionFailure> {
    let request = serde_json::from_slice::<SubscribeWorld>(line)
        .ok()
        .filter(|r| {
            r.op == "subscribe_world"
                && r.v == mag_gameplay::BRIDGE_VERSION
                && (1..=64).contains(&r.request_id.len())
                && r.request_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        });
    let Some(request) = request else {
        return write_response(
            &mut stream,
            &Response::error(String::new(), "invalid_subscription"),
        );
    };
    let (sender, receiver) = mpsc::sync_channel(32);
    static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    {
        let Some(mut guard) = vacant_subscriber_slot(&shared)? else {
            return write_response(
                &mut stream,
                &Response::error(request.request_id, "world_subscription_exists"),
            );
        };
        let mut engine = engine.lock().map_err(|_| ConnectionFailure {
            stage: ConnectionStage::WriteResponse,
            kind: io::ErrorKind::Other,
        })?;
        if !engine.accepts_subscription_world(request.world_id) {
            drop(engine);
            drop(guard);
            return write_response(
                &mut stream,
                &Response::error(request.request_id, "subscription_world_mismatch"),
            );
        }
        engine.enable_world_ticks();
        // Queue the acknowledgement before exposing the sender to the scheduler.
        sender
            .send(Response {
                v: mag_gameplay::BRIDGE_VERSION,
                request_id: request.request_id.clone(),
                ok: true,
                error: None,
                messages: Vec::new(),
                rewards: Vec::new(),
                loadout_selections: Vec::new(),
                capabilities: Vec::new(),
                match_profiles: Vec::new(),
                selected_match: None,
                match_terminal: None,
            })
            .unwrap();
        *guard = Some(WorldSubscriber {
            generation,
            world_id: request.world_id,
            request_id: request.request_id,
            sender,
        });
    }
    // Persistent subscriptions have their own single slot, separate from request workers.
    drop(slot);
    forward_subscription(stream, shared, generation, receiver)
}

// Keep the occupancy check and registration under the same lock: a new
// connection can never replace a live subscription, even during reconnects.
fn vacant_subscriber_slot(
    shared: &SharedSubscriber,
) -> Result<Option<std::sync::MutexGuard<'_, Option<WorldSubscriber>>>, ConnectionFailure> {
    let guard = shared.lock().map_err(|_| ConnectionFailure {
        stage: ConnectionStage::WriteResponse,
        kind: io::ErrorKind::Other,
    })?;
    Ok(if guard.is_none() { Some(guard) } else { None })
}

fn forward_subscription(
    mut stream: TcpStream,
    shared: SharedSubscriber,
    generation: u64,
    receiver: mpsc::Receiver<Response>,
) -> Result<(), ConnectionFailure> {
    let _owner = SubscriptionOwner {
        shared: Arc::clone(&shared),
        generation,
    };
    // A peer can send FIN while its read side continues accepting every
    // heartbeat. Poll the read side too; successful writes cannot detect that.
    // The subscription owns both TCP directions: read-side EOF ends ownership,
    // including a peer half-close that keeps consuming responses.
    stream
        .set_read_timeout(Some(Duration::from_millis(1)))
        .map_err(|error| ConnectionFailure::io(ConnectionStage::ReadRequest, error))?;
    let mut frame_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if shared
            .lock()
            .ok()
            .is_none_or(|guard| !guard.as_ref().is_some_and(|s| s.generation == generation))
        {
            return Err(ConnectionFailure {
                stage: ConnectionStage::WriteResponse,
                kind: io::ErrorKind::ConnectionAborted,
            });
        }
        let mut incoming = [0u8; 1];
        match stream.read(&mut incoming) {
            Ok(0) => {
                return Err(ConnectionFailure {
                    stage: ConnectionStage::ReadRequest,
                    kind: io::ErrorKind::ConnectionAborted,
                });
            }
            Ok(_) => {
                return Err(ConnectionFailure {
                    stage: ConnectionStage::ReadRequest,
                    kind: io::ErrorKind::InvalidData,
                });
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(ConnectionFailure::io(ConnectionStage::ReadRequest, error)),
        }
        let remaining = frame_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ConnectionFailure {
                stage: ConnectionStage::WriteResponse,
                kind: io::ErrorKind::ConnectionAborted,
            });
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(response) => {
                write_response(&mut stream, &response)?;
                frame_deadline = Instant::now() + Duration::from_secs(2);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(ConnectionFailure {
                    stage: ConnectionStage::WriteResponse,
                    kind: io::ErrorKind::ConnectionAborted,
                });
            }
        }
    }
}

struct SubscriptionOwner {
    shared: SharedSubscriber,
    generation: u64,
}
impl Drop for SubscriptionOwner {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.shared.lock()
            && guard
                .as_ref()
                .is_some_and(|s| s.generation == self.generation)
        {
            *guard = None;
        }
    }
}

fn spawn_world_scheduler(
    engine: Arc<Mutex<Engine<LogWriter>>>,
    subscriber: SharedSubscriber,
    started: Instant,
    telemetry: Option<TickRecorder>,
    budget: Option<u64>,
) {
    std::thread::spawn(move || {
        let interval = Duration::from_millis(50);
        let mut deadline = Instant::now() + interval;
        let mut heartbeat = Instant::now();
        loop {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            let now = Instant::now();
            // Skip missed deadlines: never run an unbounded burst of catch-up steps.
            deadline += interval;
            if deadline <= now {
                deadline = now + interval;
            }
            let target = subscriber.lock().ok().and_then(|s| {
                s.as_ref().map(|s| {
                    (
                        s.generation,
                        s.world_id,
                        s.request_id.clone(),
                        s.sender.clone(),
                    )
                })
            });
            let Some((generation, world_id, request_id, sender)) = target else {
                continue;
            };
            let wall = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let waiting = Instant::now();
            let start_us = started.elapsed().as_micros() as u64;
            let (response, wait_us, engine_us) = match engine.lock() {
                Ok(mut engine) => {
                    let wait = waiting.elapsed().as_micros() as u64;
                    let processing = Instant::now();
                    let response = engine.tick_world(started.elapsed(), wall, request_id);
                    (response, wait, processing.elapsed().as_micros() as u64)
                }
                Err(_) => continue,
            };
            if let Some(recorder) = telemetry.as_ref() {
                recorder.record(
                    TickSample::new(true, start_us, wait_us, engine_us, response.ok, budget)
                        .independent(),
                );
            }
            let mismatch = response.messages.iter().any(|m| m.world_id != world_id);
            if mismatch || !response.ok {
                if let Ok(mut guard) = subscriber.lock()
                    && guard.as_ref().is_some_and(|s| s.generation == generation)
                {
                    *guard = None;
                }
                continue;
            }
            let due = heartbeat.elapsed() >= Duration::from_secs(1);
            if response.messages.is_empty()
                && response.rewards.is_empty()
                && response.match_terminal.is_none()
                && !due
            {
                continue;
            }
            heartbeat = Instant::now();
            if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
                sender.try_send(response)
            {
                // Closing the stream is an explicit gap; Horizon releases affected sessions.
                if let Ok(mut guard) = subscriber.lock()
                    && guard.as_ref().is_some_and(|s| s.generation == generation)
                {
                    *guard = None;
                }
            }
        }
    });
}

#[cfg(test)]
mod subscription_tests {
    use super::*;
    use std::net::{Ipv4Addr, Shutdown};
    use std::sync::atomic::AtomicBool;

    fn response() -> Response {
        Response {
            v: mag_gameplay::BRIDGE_VERSION,
            request_id: "synthetic-world-stream".into(),
            ok: true,
            error: None,
            messages: Vec::new(),
            rewards: Vec::new(),
            loadout_selections: Vec::new(),
            capabilities: Vec::new(),
            match_profiles: Vec::new(),
            selected_match: None,
            match_terminal: None,
        }
    }

    fn sockets() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client =
            TcpStream::connect_timeout(&listener.local_addr().unwrap(), Duration::from_secs(1))
                .unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    fn register(
        shared: &SharedSubscriber,
        generation: u64,
    ) -> (SyncSender<Response>, mpsc::Receiver<Response>) {
        let (sender, receiver) = mpsc::sync_channel(32);
        let mut guard = vacant_subscriber_slot(shared)
            .ok()
            .flatten()
            .expect("vacant subscriber");
        *guard = Some(WorldSubscriber {
            generation,
            world_id: 1,
            request_id: "synthetic-world-stream".into(),
            sender: sender.clone(),
        });
        (sender, receiver)
    }

    #[test]
    fn peer_fin_releases_subscription_while_heartbeats_keep_writing() {
        let (client, server) = sockets();
        let shared = Arc::new(Mutex::new(None));
        let (sender, receiver) = register(&shared, 1);
        assert!(
            vacant_subscriber_slot(&shared).ok().flatten().is_none(),
            "a live owner must reject another registration"
        );
        let running = Arc::new(AtomicBool::new(true));
        let pulse = Arc::clone(&running);
        let heartbeat = std::thread::spawn(move || {
            while pulse.load(Ordering::Acquire) {
                if sender.try_send(response()).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let worker_shared = Arc::clone(&shared);
        let worker =
            std::thread::spawn(move || forward_subscription(server, worker_shared, 1, receiver));
        // FIN leaves the peer's read side open: all tiny heartbeats can still
        // write successfully. Only checking writes/queue timeouts misses it.
        client.shutdown(Shutdown::Write).unwrap();
        let deadline = Instant::now() + Duration::from_millis(500);
        while shared.lock().unwrap().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let released = shared.lock().unwrap().is_none();
        running.store(false, Ordering::Release);
        *shared.lock().unwrap() = None;
        heartbeat.join().unwrap();
        let _ = worker.join().unwrap();
        assert!(
            released,
            "peer FIN must release the owner despite successful heartbeat writes"
        );

        // Exercise a replacement through the same atomic slot reservation and
        // forwarding loop: its first acknowledgement must reach the new peer.
        let (mut replacement, server) = sockets();
        let (sender, receiver) = register(&shared, 2);
        sender.send(response()).unwrap();
        let worker_shared = Arc::clone(&shared);
        let worker =
            std::thread::spawn(move || forward_subscription(server, worker_shared, 2, receiver));
        let line = mag_gameplay::read_line(&mut replacement, Duration::from_secs(1)).unwrap();
        let reply: Response = serde_json::from_slice(&line).unwrap();
        assert!(reply.ok);
        assert_eq!(reply.request_id, "synthetic-world-stream");
        replacement.shutdown(Shutdown::Write).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn idle_read_timeout_keeps_the_active_subscription_exclusive() {
        let (client, server) = sockets();
        let shared = Arc::new(Mutex::new(None));
        let (_sender, receiver) = register(&shared, 1);
        let worker_shared = Arc::clone(&shared);
        let worker =
            std::thread::spawn(move || forward_subscription(server, worker_shared, 1, receiver));
        std::thread::sleep(Duration::from_millis(250));
        assert!(vacant_subscriber_slot(&shared).ok().flatten().is_none());
        client.shutdown(Shutdown::Write).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn missing_scheduler_frames_still_expire_the_subscription() {
        let (_client, server) = sockets();
        let shared = Arc::new(Mutex::new(None));
        let (_sender, receiver) = register(&shared, 1);
        let began = Instant::now();
        let failure = forward_subscription(server, Arc::clone(&shared), 1, receiver)
            .err()
            .unwrap();
        assert!(began.elapsed() >= Duration::from_secs(2));
        assert!(began.elapsed() < Duration::from_secs(3));
        assert_eq!(failure.kind, io::ErrorKind::ConnectionAborted);
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn unexpected_subscription_input_releases_its_owner() {
        let (mut client, server) = sockets();
        let shared = Arc::new(Mutex::new(None));
        let (_sender, receiver) = register(&shared, 1);
        client.write_all(b"x").unwrap();
        let failure = forward_subscription(server, Arc::clone(&shared), 1, receiver)
            .err()
            .unwrap();
        assert_eq!(failure.kind, io::ErrorKind::InvalidData);
        assert_eq!(failure.diagnostic()["category"], "invalid_framing");
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn unwinding_subscription_owner_releases_the_slot() {
        let shared = Arc::new(Mutex::new(None));
        let (_sender, _receiver) = register(&shared, 1);
        let worker_shared = Arc::clone(&shared);
        let result = std::panic::catch_unwind(move || {
            let _owner = SubscriptionOwner {
                shared: worker_shared,
                generation: 1,
            };
            panic!("synthetic subscription worker failure");
        });
        assert!(result.is_err());
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn late_cleanup_cannot_remove_a_newer_generation() {
        let shared = Arc::new(Mutex::new(None));
        let (_sender, _receiver) = register(&shared, 1);
        let old_owner = SubscriptionOwner {
            shared: Arc::clone(&shared),
            generation: 1,
        };
        // The scheduler can retire an old generation before its writer exits.
        *shared.lock().unwrap() = None;
        let (_sender, _receiver) = register(&shared, 2);
        drop(old_owner);
        assert_eq!(shared.lock().unwrap().as_ref().unwrap().generation, 2);
    }
}

#[cfg(test)]
mod runtime_profile_tests {
    use super::*;

    #[test]
    fn offline_catalog_validation_cannot_be_replaced_by_a_collision_query() {
        let error = run([
            "--validate-catalogs".to_string(),
            "--collision-query".to_string(),
            "0,0,0".to_string(),
            "1,0,0".to_string(),
        ])
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("--validate-catalogs cannot be combined with --collision-query")
        );
    }

    struct SyntheticCatalogs(PathBuf);

    impl SyntheticCatalogs {
        fn new() -> Self {
            static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "mag-synthetic-runtime-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            let files = Self(path);
            let items = serde_json::json!({"schema_version":2,"application_id":20231,
                "items":{"FactionItemList0:2":{"authored_ref":"FactionItemList0:2",
                    "faction_list":"FactionItemList0","local_ordinal":2,"wire_item_id":null,
                    "name":"synthetic_weapon","item_type":"kWeapon",
                    "definition":{"group":"weapons","name":"synthetic_weapon"}}},
                "definitions":{"weapons":{"synthetic_weapon":{"parameters":{}}}},
                "resources":{}});
            files.write("items.json", &items);
            let digest = format!(
                "{:x}",
                Sha256::digest(std::fs::read(files.0.join("items.json")).unwrap())
            );
            files.write("spawns.json", &serde_json::json!({
                "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC CLI schema test",
                "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0]},
                "match_profile":{"has_bunkers":false,"has_objectives":false},
                "maps":[{"name":"synthetic_arena","spawn_count":1,"locations":[
                    {"index":0,"token":1,"kind":"on_foot","position":{"x":10.,"y":2.,"z":12.}}
                ]}]
            }));
            let mut wire: serde_json::Value =
                serde_json::from_str(include_str!("../data/synthetic-wire-profiles.json")).unwrap();
            wire["profiles"] = serde_json::json!([wire["profiles"][1].clone()]);
            files.write("wire.json", &wire);
            files.write("blast.json", &serde_json::json!({"schema_version":1,"applications":[20230,20231],
                "character_damage":{"armor_rating_reduction":0.25,"armor_prevention_reduction":0.75,"hit_location":3,"damage_type":10}}));
            let mut provenance = serde_json::json!({"verified_binding":{"validated":true,
                "positions_compared":667,"blank_positions_compared":25,"blank_zero_empirically_verified":true}});
            for name in [
                "original_retail_elf_sha256",
                "snapshot_sha256",
                "catalog_sha256",
                "rules_sha256",
                "owner_attestation_sha256",
                "gear_slots_sha256",
                "comparator_sha256",
            ] {
                provenance[name] = serde_json::json!(if name == "catalog_sha256" {
                    digest.clone()
                } else {
                    "a".repeat(64)
                });
            }
            let mut kit = vec![None; 22];
            kit[..3].fill(Some(2_u16));
            let loadouts: Vec<_> = (0..5)
                .map(|index| {
                    serde_json::json!({
                        "update_id":1,"items":if index < 3 {kit.clone()} else {vec![None;22]},
                        "primary_ammo":if index < 3 {7} else {0}
                    })
                })
                .collect();
            files.write("bootstrap.json", &serde_json::json!({"schema_version":1,"application_id":20231,
                "provenance":provenance,
                "factions":(0..3).map(|faction|serde_json::json!({"faction":faction,"class_ref":"CharacterClassInfo0","loadouts":loadouts})).collect::<Vec<_>>()
            }));
            files.write("combat.json", &serde_json::json!({"schema_version":2,"application_id":20231,
                "status":"SYNTHETIC schema test","authority":{"bullet_damage":true,"grenade_damage":false,
                    "grenade_physics":false,"armor_reduction":true,"body_damage":true,"healing_schedule":false,"ammo_tokens":false},
                "provenance":{"catalog_sha256":digest},"bullets":{},"grenades":{},"armor":{},"healing":{},"unsupported":{},
                "body":(0..16).map(|location|serde_json::json!({"location":location,"armor_area":0,"multiplier":1.0,"source":"SYNTHETIC"})).collect::<Vec<_>>(),
                "limitations":["Synthetic schema data proves no native compatibility"]}));
            files.write("armory.json", &serde_json::json!({"schema_version":1,"application_id":20231,
                "source_sha256":{"gameplay/data/retail-items.json":digest},
                "items":[{"wire_id":2,"name":"synthetic_weapon","authored_ref":"FactionItemList0:2","available_bit":2,"factions":[0,1,2],"primary_ammo":7}],
                "factions":(0..3).map(|faction|serde_json::json!({"faction":faction,"available_item_bits":[2],"skill_ids":[],
                    "loadouts":loadouts.iter().map(|row|serde_json::json!({"update_id":row["update_id"],"items":row["items"]})).collect::<Vec<_>>()
                })).collect::<Vec<_>>()
            }));
            files.write("skills.json", &serde_json::json!({"schema_version":2,"application_id":20231,
                "source_sha256":{"gameplay/data/retail-items.json":digest},
                "table":{"Skills":{"MagCharacterSkill1":{"MagCharacterSkill":{"Green":{"Available":true}}}}}}));
            files.write("fall.json", &serde_json::json!({"schema_version":1,"source_build":"retail","application_id":20231,
                "min_height":3.0,"damage_factor":4.0,"minimum_damage_exclusive":2.0,"jump_fall_state":1,"damage_type":5,"damage_source_type":1}));
            files
        }

        fn write(&self, name: &str, value: &serde_json::Value) {
            std::fs::write(self.0.join(name), serde_json::to_vec(value).unwrap()).unwrap();
        }

        fn read(&self, name: &str) -> serde_json::Value {
            serde_json::from_slice(&std::fs::read(self.0.join(name)).unwrap()).unwrap()
        }

        fn args(&self) -> Vec<String> {
            let mut args = vec![
                "--application-id".into(),
                "20231".into(),
                "--validate-catalogs".into(),
            ];
            for (flag, name) in [
                ("--item-catalog", "items.json"),
                ("--spawn-catalog", "spawns.json"),
                ("--wire-profiles", "wire.json"),
                ("--character-blast-rules", "blast.json"),
                ("--bootstrap-loadouts", "bootstrap.json"),
                ("--retail-combat-metadata", "combat.json"),
                ("--retail-armory-catalog", "armory.json"),
                ("--skill-catalog", "skills.json"),
                ("--fall-damage-rules", "fall.json"),
            ] {
                args.push(flag.into());
                args.push(self.0.join(name).to_str().unwrap().into());
            }
            args
        }
    }

    impl Drop for SyntheticCatalogs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn offline_cli_validates_explicit_bot_names_without_opening_logs() {
        let files = SyntheticCatalogs::new();
        files.write(
            "names.json",
            &serde_json::json!({"entries":[{"name":"Synthetic Pilot"}]}),
        );
        let args = || {
            let mut args = files.args();
            args.extend([
                "--bot-names".to_string(),
                files.0.join("names.json").display().to_string(),
            ]);
            args
        };
        assert!(run(args()).is_ok());
        let mut insufficient = args();
        insufficient.extend(["--bot-fill-total".to_string(), "64".to_string()]);
        assert!(
            run(insufficient)
                .unwrap_err()
                .to_string()
                .contains("insufficient capacity")
        );
        files.write("names.json", &serde_json::json!({"entries":[{"name":""}]}));
        assert!(run(args()).unwrap_err().to_string().contains("bot names"));
        std::fs::remove_file(files.0.join("names.json")).unwrap();
        assert!(run(args()).is_err());
        let mut relative = files.args();
        relative.extend(["--bot-names".to_string(), "names.json".to_string()]);
        assert!(run(relative).unwrap_err().to_string().contains("absolute"));
    }

    #[test]
    fn offline_retail_cli_validates_only_explicit_synthetic_resources() {
        let files = SyntheticCatalogs::new();
        assert!(run(files.args()).is_ok());
        assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 9);
    }

    #[test]
    fn ground_vehicle_cli_requires_explicit_qualified_retail_bindings() {
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["game_type"] = serde_json::json!(1);
        spawns["maps"][0]["owner_faction"] = serde_json::json!(0);
        spawns["maps"][0]["runtime_indices_verified"] = serde_json::json!(true);
        spawns["maps"][0]["runtime_indices_basis"] = serde_json::json!("SYNTHETIC identity proof");
        files.write("spawns.json", &spawns);
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(files.0.join("spawns.json")).unwrap())
        );
        files.write("vehicles.json", &serde_json::json!({"format":"mag_ground_vehicles","schema_version":1,
            "source":{"spawn_catalog_sha256":digest,"native_elf_sha256":"a".repeat(64),
                "native_binding_sha256":"b".repeat(64),"vehicle_properties_sha256":"c".repeat(64)},
            "vehicles":[{"index":0,"spawn_point_index":0,"spawn_point_token":spawns["maps"][0]["locations"][0]["token"],
                "vehicle_type_token":123,"faction":spawns["maps"][0]["owner_faction"],"team":1,"modifier":0,
                "position":{"x":100.,"y":3.,"z":100.},"rotation":[0.,0.,0.,1.],"health_quarter_points":600}]}));
        let mut args = files.args();
        args.extend([
            "--ground-vehicles".into(),
            files.0.join("vehicles.json").display().to_string(),
        ]);
        assert!(run(args.clone()).is_ok());
        spawns["maps"][0]["runtime_indices_verified"] = serde_json::json!(false);
        files.write("spawns.json", &spawns);
        assert!(
            run(args)
                .unwrap_err()
                .to_string()
                .contains("verified_spawn_pool")
        );
    }

    #[test]
    fn dynamic_body_cli_validates_explicit_resources_without_opening_logs() {
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["game_type"] = serde_json::json!(1);
        files.write("spawns.json", &spawns);
        let body = serde_json::json!({"format":"mag_dynamic_bodies","schema_version":1,
         "source":{"native_binding_sha256":"a".repeat(64),"shape_data_sha256":"b".repeat(64)},
         "runtime_bindings":{"character_template":"invented","vehicles":[]},
         "templates":[{"id":"invented","kind":"character","roots":[{"id":"body","layer":21,"dead_layer":6,"translation":[0.,0.,0.],"rotation":"identity","shape":{"kind":"sphere","radius":0.25}}]}]});
        files.write("bodies.json", &body);
        let args = || {
            let mut v = files.args();
            v.extend([
                "--dynamic-bodies".to_owned(),
                files.0.join("bodies.json").display().to_string(),
            ]);
            v
        };
        assert!(run(args()).is_ok());
        let mut bad = body.clone();
        bad["runtime_bindings"]["character_template"] = serde_json::json!("missing");
        files.write("bodies.json", &bad);
        assert!(
            run(args())
                .unwrap_err()
                .to_string()
                .contains("dynamic_body_bindings")
        );
        let mut relative = files.args();
        relative.extend(["--dynamic-bodies".to_owned(), "bodies.json".to_owned()]);
        assert!(run(relative).unwrap_err().to_string().contains("absolute"));
        assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 10);
    }

    fn synthetic_match_profile(files: &SyntheticCatalogs) -> PathBuf {
        files.write("profile-spawns.json", &serde_json::json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC CLI registry",
            "active":{"map_name":"synthetic_home","squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":0,"auto_assign_teams":true,"team_capacity":1,"squad_capacity":1,"attacking_faction":0,"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":"synthetic_home","owner_faction":1,"spawn_count":1,
                "locations":[{"index":0,"token":1,"kind":"on_foot","position":{"x":10.,"y":2.,"z":12.}}],
                "squads":[{"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0],"default_spawn":0},
                {"index":16,"team":1,"platoon":0,"squad":0,"faction":1,"available":[0],"default_spawn":0}]}]}));
        files.write("profiles.json", &serde_json::json!({"format":"mag_match_profiles_v1","profiles":[{"profile_id":"synthetic-profile","spawn_catalog":files.0.join("profile-spawns.json")}]}));
        files.0.join("profiles.json")
    }

    #[test]
    fn offline_registry_validation_is_explicit_and_still_requires_base_inputs() {
        let files = SyntheticCatalogs::new();
        let manifest = synthetic_match_profile(&files);
        let mut args = files.args();
        args.extend(["--match-profiles".into(), manifest.display().to_string()]);
        assert!(run(args.clone()).is_ok());
        let spawn_flag = args.iter().position(|s| s == "--spawn-catalog").unwrap();
        args.drain(spawn_flag..=spawn_flag + 1);
        assert!(
            run(args)
                .unwrap_err()
                .to_string()
                .contains("--spawn-catalog")
        );
    }

    #[test]
    fn invalid_registry_fails_before_creating_log_or_listener() {
        let files = SyntheticCatalogs::new();
        let manifest = synthetic_match_profile(&files);
        let mut broken = files.read("profiles.json");
        broken["profiles"][0]["spawn_catalog"] = serde_json::json!(files.0.join("missing.json"));
        files.write("profiles.json", &broken);
        let mut args = files.args();
        args.retain(|s| s != "--validate-catalogs");
        args.extend([
            "--match-profiles".into(),
            manifest.display().to_string(),
            "--log".into(),
            files.0.join("should-not-exist.jsonl").display().to_string(),
        ]);
        assert!(run(args).is_err());
        assert!(!files.0.join("should-not-exist.jsonl").exists());
    }

    #[test]
    fn registry_rejects_unswitchable_world_extras_before_loading() {
        for flags in [
            vec!["--bot-population", "/synthetic/missing.json"],
            vec!["--bot-fill-total", "1"],
            vec!["--development-dummy"],
            vec!["--leadership-passives", "/synthetic/missing.json"],
            vec!["--bunker-catalog", "/synthetic/missing.json"],
        ] {
            let mut args = vec![
                "--application-id".to_string(),
                "20231".into(),
                "--match-profiles".into(),
                "/synthetic/profiles.json".into(),
            ];
            args.extend(flags.into_iter().map(str::to_owned));
            assert!(run(args).is_err());
        }
    }

    #[test]
    fn character_damage_rules_are_optional_explicit_and_bound_to_exact_spawn_bytes() {
        let files = SyntheticCatalogs::new();
        files.write(
            "character-damage.json",
            &serde_json::json!({"invalid":"not implicitly loaded"}),
        );
        assert!(run(files.args()).is_ok());
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(files.0.join("spawns.json")).unwrap())
        );
        files.write(
            "character-damage.json",
            &serde_json::json!({"schema_version":1,"application_id":20231,
            "map_name":"synthetic_arena","game_type":2,"friendly_fire_multiplier":0.375,
            "provenance":{"spawn_catalog_sha256":digest,"source":"synthetic"}}),
        );
        let args = || {
            let mut args = files.args();
            args.extend([
                "--character-damage-rules".into(),
                files.0.join("character-damage.json").display().to_string(),
            ]);
            args
        };
        assert!(run(args()).is_ok());
        // Same parsed match identity, different original file bytes: fail clearly.
        let mut bytes = std::fs::read(files.0.join("spawns.json")).unwrap();
        bytes.push(b' ');
        std::fs::write(files.0.join("spawns.json"), bytes).unwrap();
        assert!(
            run(args())
                .unwrap_err()
                .to_string()
                .contains("match_mismatch")
        );
        let mut relative = files.args();
        relative.extend([
            "--character-damage-rules".into(),
            "character-damage.json".into(),
        ]);
        assert!(run(relative).unwrap_err().to_string().contains("absolute"));
    }

    #[test]
    fn incapacitation_is_optional_and_requires_explicit_disk_body_data_when_enabled() {
        let files = SyntheticCatalogs::new();
        assert!(run(files.args()).is_ok());
        files.write(
            "incapacitation.json",
            &serde_json::json!({"schema_version":1,
            "incapacitated_damage_threshold_hp":83.0,"incapacitated_duration_seconds":7.0,
            "excluded_character_states":[22]}),
        );
        let args = || {
            let mut args = files.args();
            args.extend([
                "--incapacitation-rules".into(),
                files.0.join("incapacitation.json").display().to_string(),
            ]);
            args
        };
        assert!(
            run(args())
                .unwrap_err()
                .to_string()
                .contains("explicit body multipliers")
        );
        let mut combat = files.read("combat.json");
        for body in combat["body"].as_array_mut().unwrap() {
            body["incapacitated_multiplier"] = serde_json::json!(0.37);
        }
        files.write("combat.json", &combat);
        assert!(run(args()).is_ok());
        let mut relative = files.args();
        relative.extend([
            "--incapacitation-rules".into(),
            "incapacitation.json".into(),
        ]);
        assert!(
            run(relative)
                .unwrap_err()
                .to_string()
                .contains("absolute path")
        );
        files.write(
            "incapacitation.json",
            &serde_json::json!({"schema_version":1}),
        );
        assert!(run(args()).is_err());
    }

    #[test]
    fn offline_retail_cli_rejects_unresolved_raw_items_in_armory_bootstrap() {
        let files = SyntheticCatalogs::new();
        let mut items = files.read("items.json");
        items["items"]["FactionItemList0:2"]["definition"] = serde_json::Value::Null;
        files.write("items.json", &items);
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(files.0.join("items.json")).unwrap())
        );
        for (name, pointer) in [
            ("bootstrap.json", "/provenance/catalog_sha256"),
            ("combat.json", "/provenance/catalog_sha256"),
            (
                "armory.json",
                "/source_sha256/gameplay~1data~1retail-items.json",
            ),
            (
                "skills.json",
                "/source_sha256/gameplay~1data~1retail-items.json",
            ),
        ] {
            let mut value = files.read(name);
            *value.pointer_mut(pointer).unwrap() = serde_json::json!(digest);
            files.write(name, &value);
        }
        assert!(
            run(files.args())
                .unwrap_err()
                .to_string()
                .contains("unresolved_saved_item")
        );
    }

    #[test]
    fn offline_retail_cli_requires_selected_collision_world_to_match_the_map() {
        let files = SyntheticCatalogs::new();
        let mut world = serde_json::json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"different_synthetic_arena","build":"synthetic",
                "coverage":"provisional","provenance":["Synthetic CLI test"],"unknowns":["Not native geometry"]},
            "instances":[{"id":"synthetic-wall","vertices_xyz":[[2.,-2.,-2.],[2.,2.,-2.],[2.,0.,2.]],"triangles":[[0,1,2]]}]
        });
        files.write("world.json", &world);
        let mut args = files.args();
        args.extend([
            "--collision-world".into(),
            files.0.join("world.json").to_str().unwrap().into(),
        ]);
        assert!(
            run(args.clone())
                .unwrap_err()
                .to_string()
                .contains("collision world map does not match")
        );
        world["metadata"]["map_name"] = serde_json::json!("synthetic_arena");
        files.write("world.json", &world);
        assert!(run(args).is_ok());
    }

    #[test]
    fn offline_retail_cli_rejects_skill_profile_mismatch() {
        let files = SyntheticCatalogs::new();
        let mut skills = files.read("skills.json");
        skills["schema_version"] = serde_json::json!(1);
        skills["application_id"] = serde_json::json!(20230);
        files.write("skills.json", &skills);
        let error = run(files.args()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("skill catalog application does not match")
        );
    }

    #[test]
    fn offline_retail_cli_rejects_item_projection_hash_mismatch() {
        let files = SyntheticCatalogs::new();
        let mut combat = files.read("combat.json");
        combat["provenance"]["catalog_sha256"] = serde_json::json!("0".repeat(64));
        files.write("combat.json", &combat);
        let error = run(files.args()).unwrap_err();
        assert!(error.to_string().contains("retail combat does not match"));
    }

    #[test]
    fn retail_cli_keeps_bunker_and_alpha_inputs_unsupported() {
        for flag in [
            "--structure-catalog",
            "--bunker-catalog",
            "--bunker-exit-policy",
        ] {
            let files = SyntheticCatalogs::new();
            let mut args = files.args();
            args.extend([
                flag.into(),
                files
                    .0
                    .join("unused-synthetic.json")
                    .to_str()
                    .unwrap()
                    .into(),
            ]);
            let error = run(args).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("retail structure/bunker authority is unverified"),
                "{flag}: {error}"
            );
        }
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["has_bunkers"] = serde_json::json!(true);
        files.write("spawns.json", &spawns);
        assert!(
            run(files.args()).unwrap_err().to_string().contains(
                "retail bunker features are unsupported without verified retail authority"
            )
        );
    }

    #[test]
    fn offline_retail_cli_requires_catalog_for_enabled_objectives() {
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["has_objectives"] = serde_json::json!(true);
        files.write("spawns.json", &spawns);
        let error = run(files.args()).unwrap_err();
        assert!(error.to_string().contains("objective catalog required"));
    }

    #[test]
    fn offline_retail_cli_requires_bootstrap_for_all_factions() {
        let files = SyntheticCatalogs::new();
        let mut bootstrap = files.read("bootstrap.json");
        bootstrap["factions"].as_array_mut().unwrap().pop();
        files.write("bootstrap.json", &bootstrap);
        let error = run(files.args()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("retail bootstrap must include all supported factions")
        );
    }

    #[test]
    fn offline_retail_cli_does_not_create_runtime_logs() {
        let files = SyntheticCatalogs::new();
        let mut args = files.args();
        for (flag, name) in [("--log", "runtime.log"), ("--tick-telemetry", "ticks.log")] {
            args.push(flag.into());
            args.push(files.0.join(name).to_str().unwrap().into());
        }
        assert!(
            run(args)
                .unwrap_err()
                .to_string()
                .contains("does not open logs")
        );
        assert!(!files.0.join("runtime.log").exists());
        assert!(!files.0.join("ticks.log").exists());
    }

    #[test]
    fn log_rotation_flags_are_validated_before_any_log_opens() {
        let files = SyntheticCatalogs::new();
        let with = |extra: &[&str]| {
            let mut args = files.args();
            args.extend(extra.iter().map(|value| value.to_string()));
            run(args).unwrap_err().to_string()
        };
        assert!(with(&["--log-retention-s", "3600"]).contains("requires --log-segment-bytes"));
        assert!(with(&["--log-segment-bytes", "1024"]).contains("log_segment_bytes_too_small"));
        assert!(
            with(&["--log-segment-bytes", "1048576", "--log-retention-s", "0"])
                .contains("log_retention_must_be_positive")
        );
        assert!(with(&["--log-segment-bytes", "x"]).contains("invalid --log-segment-bytes"));
        assert!(
            with(&[
                "--log-segment-bytes",
                "1048576",
                "--log-retention-s",
                "3600"
            ])
            .contains("does not open logs")
        );
    }

    #[test]
    fn application_selection_is_explicit_and_bounded() {
        assert_eq!(parse_application_id("20230").unwrap(), 20230);
        assert_eq!(parse_application_id("20231").unwrap(), 20231);
        for invalid in ["", "0", "20232", " 20231", "20231 "] {
            assert!(parse_application_id(invalid).is_err());
        }
    }
    #[test]
    fn retail_paths_cannot_fall_back_to_sibling_defaults() {
        assert!(validate_runtime_paths(Some(20231), &[("--item-catalog", None)]).is_err());
        assert!(
            validate_runtime_paths(
                Some(20231),
                &[("--item-catalog", Some(Path::new("relative.json")))]
            )
            .is_err()
        );
        assert!(
            validate_runtime_paths(
                Some(20231),
                &[("--item-catalog", Some(Path::new("/catalogs/items.json")))]
            )
            .is_ok()
        );
        assert!(validate_runtime_paths(None, &[("legacy-default", None)]).is_ok());
    }
    #[test]
    fn retail_projections_must_identify_the_exact_item_bytes() {
        let temporary = std::env::temp_dir().join(format!(
            "synthetic-retail-links-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&temporary).unwrap();
        let items = temporary.join("items.json");
        let projection = temporary.join("projection.json");
        std::fs::write(&items, b"synthetic item bytes").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"synthetic item bytes"));
        std::fs::write(
            &projection,
            serde_json::to_vec(&serde_json::json!({"provenance":{"catalog_sha256":digest}}))
                .unwrap(),
        )
        .unwrap();
        let matched = validate_item_catalog_links(
            &items,
            &[("projection", &projection, "/provenance/catalog_sha256")],
        )
        .is_ok();
        std::fs::write(&items, b"different synthetic bytes").unwrap();
        let mismatched = validate_item_catalog_links(
            &items,
            &[("projection", &projection, "/provenance/catalog_sha256")],
        )
        .is_err();
        std::fs::write(&projection, b"{}").unwrap();
        let missing = validate_item_catalog_links(
            &items,
            &[("projection", &projection, "/provenance/catalog_sha256")],
        )
        .is_err();
        std::fs::remove_dir_all(temporary).unwrap();
        assert!(matched && mismatched && missing);
    }
}

#[cfg(test)]
mod initial_spawn_delay_flag_tests {
    use super::parse_team_delays;

    #[test]
    fn one_value_applies_to_both_teams_and_pairs_are_per_team() {
        assert_eq!(parse_team_delays("10000"), Some([10000, 10000]));
        assert_eq!(parse_team_delays("10000,15000"), Some([10000, 15000]));
        for invalid in ["", "x", "10000,", "1,2,3", "-5", "10000;15000"] {
            assert_eq!(parse_team_delays(invalid), None, "{invalid:?}");
        }
    }
}
