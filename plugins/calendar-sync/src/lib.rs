use exports::satchel::plugin::plugin::Guest;

wit_bindgen::generate!({
    path: "../../plugin-sdk/wit",
    world: "satchel-plugin",
});

struct CalendarPlugin;

impl Guest for CalendarPlugin {
    fn run_command(command_id: String, payload: String) -> Result<String, String> {
        Ok(format!("calendar command={command_id} payload={payload}"))
    }
}

export!(CalendarPlugin);
