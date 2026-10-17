use exports::satchel::plugin::plugin::Guest;

wit_bindgen::generate!({
    path: "../../plugin-sdk/wit",
    world: "satchel-plugin",
});

struct AutoUnsubscribePlugin;

impl Guest for AutoUnsubscribePlugin {
    fn run_command(command_id: String, payload: String) -> Result<String, String> {
        Ok(format!(
            "unsubscribe command={command_id} payload={payload}"
        ))
    }
}

export!(AutoUnsubscribePlugin);
