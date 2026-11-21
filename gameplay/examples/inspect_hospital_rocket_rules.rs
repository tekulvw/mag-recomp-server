//! Inspect source-qualified retail hospital rocket rules offline.
use mag_gameplay::{
    hospital_rockets::Catalog, item_catalog::ItemCatalog, retail_combat::RetailCombatCatalog,
};
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [
        rules,
        items,
        character_blast,
        combat,
        hospital_rules,
        native_elf_sha,
    ] = args.as_slice()
    else {
        return Err(
            "usage: inspect_hospital_rocket_rules RULES ITEMS CHARACTER_BLAST COMBAT HOSPITAL_RULES NATIVE_ELF_SHA"
                .into(),
        );
    };

    let items = ItemCatalog::load_for_application(items, character_blast, 20231)?
        .with_retail_combat(RetailCombatCatalog::load(combat)?);
    let catalog = Catalog::from_bytes(
        &fs::read(rules)?,
        &items,
        &fs::read(hospital_rules)?,
        native_elf_sha,
    )?;

    println!("Offline source inspection. This command does not start an Engine or enable firing.");
    println!("Native ELF SHA-256 {native_elf_sha}");
    for binding in catalog.variants() {
        let variant = catalog
            .secondary_for(binding.faction, binding.secondary_item_id)
            .ok_or("qualified hospital secondary lookup failed")?;
        println!(
            "Faction {} secondary {} uses {} ({:#010x})",
            variant.faction, variant.secondary_item_id, variant.ammo_name, variant.ammo_token
        );
        println!("  Weapon {:?}", variant.weapon);
        println!("  Muzzle {:?}", variant.muzzle);
        println!("  Flight {:?}", variant.flight);
        println!("  Blast {:?}", variant.blast);
    }
    Ok(())
}
