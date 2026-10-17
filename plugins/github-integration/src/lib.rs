use exports::satchel::plugin::plugin::Guest;

wit_bindgen::generate!({
    path: "../../plugin-sdk/wit",
    world: "satchel-plugin",
});

struct GithubPlugin;

impl Guest for GithubPlugin {
    fn run_command(command_id: String, payload: String) -> Result<String, String> {
        Ok(format!("github command={command_id} payload={payload}"))
    }
}

export!(GithubPlugin);
