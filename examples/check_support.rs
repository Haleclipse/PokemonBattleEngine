//! Prints how the engine classifies named moves, abilities and items in a generation (supported
//! or not, and the unmodelled Showdown callbacks): the quick check before putting a set into a
//! scenario or a corpus.
//!
//! Run: cargo run --example check_support -- [--gen N] "Life Orb" "Speed Boost" "Body Slam"
use pokemon_battle_engine::dex::{DexResolver, PrimaryEffect};
use pokemon_battle_engine::spec::Generation;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut generation = Generation::Gen9;
    if args.first().map(String::as_str) == Some("--gen") && args.len() >= 2 {
        generation = match args[1].as_str() {
            "1" => Generation::Gen1,
            "2" => Generation::Gen2,
            "3" => Generation::Gen3,
            "4" => Generation::Gen4,
            "5" => Generation::Gen5,
            "6" => Generation::Gen6,
            "7" => Generation::Gen7,
            "8" => Generation::Gen8,
            _ => Generation::Gen9,
        };
        args.drain(..2);
    }
    let dex = DexResolver::resolve(generation);
    for name in args {
        if let Some(m) = dex.move_by_name(&name) {
            match &m.primary_effect {
                PrimaryEffect::Unsupported => println!("UNSUPP move {name}: {:?}", m.unsupported),
                _ => println!("ok     move {name}"),
            }
            // `CHECK_VERBOSE=1`: the resolved move as the engine sees it.
            if std::env::var_os("CHECK_VERBOSE").is_some() {
                println!("{m:#?}");
            }
        } else if let Some(a) = dex.ability_by_name(&name) {
            println!(
                "{} ability {name}: {:?}",
                if a.implemented { "ok    " } else { "UNSUPP" },
                a.missing_callbacks
            );
            if std::env::var_os("CHECK_VERBOSE").is_some() {
                println!("{a:#?}");
            }
        } else if let Some(i) = dex.item_by_name(&name) {
            println!(
                "{} item {name}: {:?}",
                if i.implemented { "ok    " } else { "UNSUPP" },
                i.missing_callbacks
            );
            if std::env::var_os("CHECK_VERBOSE").is_some() {
                println!("{i:#?}");
                println!("item_user: {:?}", i.item_user);
            }
        } else if dex.species_by_name(&name).is_some() {
            println!("ok     species {name}");
        } else {
            println!("MISSING {name}");
        }
    }
}
