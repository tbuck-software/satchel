#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args = std::env::args_os().collect::<Vec<_>>();
    if args
        .get(1)
        .is_some_and(|arg| arg == "--verify-plugin-packages")
    {
        if args.len() != 3 {
            eprintln!("Usage: satchel --verify-plugin-packages <resource-plugins-directory>");
            std::process::exit(2);
        }
        match satchel_lib::plugins::registry::PluginRegistry::verify_release_packages(
            std::path::Path::new(&args[2]),
        ) {
            Ok(plugins) => println!("Verified {} signed release plugins", plugins.len()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    satchel_lib::run();
}
