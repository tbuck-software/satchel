use std::path::PathBuf;
use std::time::Duration;

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../plugin-sdk/wit",
    world: "satchel-plugin",
});

struct SimWasiState {
    table: ResourceTable,
    wasi: WasiCtx,
}

impl WasiView for SimWasiState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let component_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "Usage: plugin_sim <component.wasm> <command> <payload>".to_string())?;
    let command = args
        .next()
        .ok_or_else(|| "Usage: plugin_sim <component.wasm> <command> <payload>".to_string())?;
    let payload = args
        .next()
        .ok_or_else(|| "Usage: plugin_sim <component.wasm> <command> <payload>".to_string())?;

    let mut config = Config::new();
    config.wasm_component_model(true);
    config.consume_fuel(true);
    config.epoch_interruption(true);
    let engine = Engine::new(&config).map_err(|err| format!("engine init failed: {err}"))?;
    let component = Component::from_file(&engine, &component_path)
        .map_err(|err| format!("component load failed: {err}"))?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|err| format!("wasi linker setup failed: {err}"))?;
    let mut store = Store::new(
        &engine,
        SimWasiState {
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new().build(),
        },
    );
    store
        .set_fuel(5_000_000)
        .map_err(|err| format!("fuel setup failed: {err}"))?;
    store.set_epoch_deadline(1);
    store.epoch_deadline_trap();

    let timeout_engine = engine.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        timeout_engine.increment_epoch();
    });

    let bindings = SatchelPlugin::instantiate(&mut store, &component, &linker)
        .map_err(|err| format!("component instantiate failed: {err}"))?;
    let output = bindings
        .satchel_plugin_plugin()
        .call_run_command(&mut store, &command, &payload)
        .map_err(|err| format!("plugin command failed: {err}"))?
        .map_err(|err| format!("plugin returned error: {err}"))?;
    println!("{output}");
    Ok(())
}
