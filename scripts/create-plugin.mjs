import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pluginName = process.argv[2];

if (!pluginName || !/^[a-z0-9][a-z0-9-]*$/.test(pluginName)) {
  process.stderr.write('Usage: pnpm run plugin:create -- my-plugin-name\n');
  process.exit(1);
}

const packageName = pluginName.startsWith('satchel-plugin-')
  ? pluginName
  : `satchel-plugin-${pluginName}`;
const pluginDir = join(repoRoot, 'plugins', packageName.replace(/^satchel-plugin-/, ''));

if (existsSync(pluginDir)) {
  process.stderr.write(`Plugin already exists: ${pluginDir}\n`);
  process.exit(1);
}

mkdirSync(join(pluginDir, 'src'), { recursive: true });

writeFileSync(
  join(pluginDir, 'Cargo.toml'),
  `[package]
name = "${packageName}"
version = "0.1.0"
edition = "2021"

[package.metadata.satchel]
api_version = "1.0"
capabilities = ["read_threads"]
events = []
ui_slots = []
trust = "local_unsigned"

[lib]
crate-type = ["cdylib", "rlib"]
test = false
doctest = false

[dependencies]
satchel-plugin-sdk = { path = "../../plugin-sdk" }
wit-bindgen = "0.53.1"
`,
);

writeFileSync(
  join(pluginDir, 'src', 'lib.rs'),
  `use exports::satchel::plugin::plugin::Guest;

wit_bindgen::generate!({
    path: "../../plugin-sdk/wit",
    world: "satchel-plugin",
});

struct Plugin;

impl Guest for Plugin {
    fn run_command(command_id: String, payload: String) -> Result<String, String> {
        match command_id.as_str() {
            "transform-message" => Ok(payload),
            "handle-event" => Ok(String::new()),
            other => Err(format!("command not implemented: {other}")),
        }
    }
}

export!(Plugin);
`,
);

process.stdout.write(`Created ${packageName} in ${pluginDir}\n`);
