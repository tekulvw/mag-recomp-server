use mag_gameplay::{
    active_abilities::Rules,
    active_ability_state::NativeAbilityData,
    strategic_abilities::{
        SensorScan, StrategicAbility as Ability, StrategicStart, StrategicState, spawn_interval_ms,
    },
};

fn rules() -> Rules {
    let modes = [64, 128, 256]
        .into_iter()
        .flat_map(|players| {
            ["attacker", "defender"].map(move |side| {
                let strategic = if players != 256 {
                    Vec::new()
                } else {
                    vec![
                        "SignalsJamming", "SensorSweep", "TacticalStrike",
                        if side == "attacker" { "Blockade" } else { "RapidRedeploy" },
                    ]
                };
                serde_json::json!({"players":players,"side":side,"squad":[],"platoon":[],"strategic":strategic})
            })
        })
        .collect::<Vec<_>>();
    let strategic = [
        ("SignalsJamming", 200, None),
        ("SensorSweep", 200, None),
        ("TacticalStrike", 30, None),
        ("Blockade", 200, Some(0.25)),
        ("RapidRedeploy", 200, Some(-0.25)),
    ]
    .into_iter()
    .map(|(name, duration, modifier)| {
        serde_json::json!({"ability":name,"source_member":"invented","source_sha256":"ba".repeat(32),
            "cooldown_ms":3_000,"duration_ms":duration,"spawn_interval_modifier":modifier})
    })
    .collect::<Vec<_>>();
    serde_json::from_value(serde_json::json!({
        "schema_version":1,"application_id":20231,"rules_sha256":"ab".repeat(32),
        "source_archive_sha256":"bc".repeat(32),"global_source_member":"invented","global_source_sha256":"cd".repeat(32),
        "command_global_cooldown_ms":10,"strategic_global_cooldown_ms":10,
        "authority":{"effects":false,"target_validation":false,"ability_replication":false},
        "modes":modes,"commands":[],"strategic":strategic,
    }))
    .unwrap()
}

#[test]
fn company_availability_and_cooldowns_are_separate_and_use_native_time_zero() {
    let mut state = StrategicState::new(&rules()).unwrap();
    assert_eq!(Ability::from_code(7), Ok(Ability::RapidRedeploy));
    assert!(Ability::from_code(9).is_err());
    assert_eq!(
        state.company(0).unwrap().available_mask,
        Ability::Blockade.mask()
            | Ability::SignalsJamming.mask()
            | Ability::SensorSweep.mask()
            | Ability::TacticalStrike.mask()
    );
    assert!(state.activate(1, 0, Ability::Blockade, 10).is_err());
    assert!(state.activate(2, 1, Ability::Blockade, 10).is_err());
    assert!(state.activate(2, 0, Ability::Blockade, 9).is_err());
    assert_eq!(
        state.activate(2, 0, Ability::Blockade, 10),
        Ok(StrategicStart::None)
    );
    assert!(state.activate(2, 0, Ability::SensorSweep, 19).is_err());
    assert_eq!(
        state.activate(2, 1, Ability::SensorSweep, 19),
        Ok(StrategicStart::None)
    );

    let (mut state, _) = state.prepare_tick(211).unwrap();
    assert!(state.activate(2, 0, Ability::Blockade, 3009).is_err());
    assert!(state.activate(2, 0, Ability::Blockade, 3010).is_ok());
}

#[test]
fn jamming_lock_and_spawn_modifiers_are_scoped_to_the_qualified_team() {
    let mut state = StrategicState::new(&rules()).unwrap();
    assert!(state.activate(2, 0, Ability::Blockade, 10).is_ok());
    assert_eq!(state.spawn_interval_modifier(0), 0.);
    assert_eq!(state.spawn_interval_modifier(1), 0.25);
    assert_eq!(
        spawn_interval_ms(145, state.spawn_interval_modifier(1)),
        181
    );
    assert!(state.activate(2, 1, Ability::RapidRedeploy, 10).is_ok());
    assert_eq!(state.spawn_interval_modifier(1), 0.);
    assert!(state.activate(2, 0, Ability::SignalsJamming, 20).is_ok());
    assert!(!state.commands_locked(0));
    assert!(state.commands_locked(1));
    assert!(state.activate(2, 1, Ability::SensorSweep, 100).is_err());

    let (state, _) = state.prepare_tick(220).unwrap();
    assert!(state.commands_locked(1));
    let (mut state, _) = state.prepare_tick(221).unwrap();
    assert!(!state.commands_locked(1));
    assert_eq!(
        state.activate(2, 1, Ability::TacticalStrike, 222),
        Ok(StrategicStart::RefreshCommands { team: 1 })
    );
}

#[test]
fn sensor_sweep_wraps_member_slots_and_sends_the_last_native_scan_on_expiry() {
    let mut state = StrategicState::new(&rules()).unwrap();
    assert!(state.activate(2, 0, Ability::SensorSweep, 20).is_ok());
    assert!(state.activate(2, 1, Ability::SensorSweep, 20).is_ok());
    let mut scans = Vec::new();
    for ms in 20..38 {
        let (next, scan) = state.prepare_tick(ms).unwrap();
        scans.push(scan);
        state = next;
    }
    assert_eq!(
        scans[0],
        [
            Some(SensorScan { squad: 16 }),
            Some(SensorScan { squad: 0 })
        ]
    );
    assert_eq!(
        scans[15],
        [
            Some(SensorScan { squad: 31 }),
            Some(SensorScan { squad: 15 })
        ]
    );
    assert_eq!(scans[16], scans[0]);

    let (state, scan) = state.prepare_tick(221).unwrap();
    assert!(scan.iter().all(Option::is_some));
    let company = state.company(0).unwrap();
    assert_eq!(company.active_mask, 0);
    assert_eq!(
        company.data[0],
        Some(NativeAbilityData {
            activate_time_ms: 20,
            enabled_at_start: true,
            ..Default::default()
        })
    );
    assert_eq!(state.prepare_tick(222).unwrap().1, [None, None]);
}

#[test]
fn required_spawn_modifier_cannot_be_inferred_from_a_legacy_input() {
    let mut source = rules();
    source
        .strategic
        .iter_mut()
        .find(|r| r.ability == "Blockade")
        .unwrap()
        .spawn_interval_modifier = None;
    assert!(StrategicState::new(&source).is_err());
}

#[test]
fn company_percentage_clock_uses_native_u32_numerator() {
    let mut state = StrategicState::new(&rules()).unwrap();
    let wraps_percent_at = u32::MAX / 100 + 1;
    assert_eq!(
        state.activate(2, 0, Ability::Blockade, wraps_percent_at),
        Err("strategic_global_cooldown")
    );
    assert!(
        state
            .activate(2, 0, Ability::Blockade, wraps_percent_at + 10)
            .is_ok()
    );
}
