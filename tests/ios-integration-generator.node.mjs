import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";

const sourceRoot = resolve(process.cwd());
let root = sourceRoot;
if (!existsSync(join(root, "src-tauri/gen/apple/project.yml"))) {
  root = mkdtempSync(join(tmpdir(), "satchel-ios-generator-"));
  cpSync(
    join(sourceRoot, "src/utils/fixtures/generated-apple"),
    join(root, "src-tauri/gen/apple"),
    { recursive: true },
  );
  mkdirSync(join(root, "src-tauri/ios"), { recursive: true });
  cpSync(
    join(sourceRoot, "src-tauri/ios"),
    join(root, "src-tauri/ios"),
    { recursive: true },
  );
  // The generator reads the deep-link registration from the real configuration.
  cpSync(
    join(sourceRoot, "src-tauri/tauri.conf.json"),
    join(root, "src-tauri/tauri.conf.json"),
  );
}
const apple = join(root, "src-tauri/gen/apple");
const generator = join(sourceRoot, "scripts/apply-ios-integration.mjs");
function apply() {
  execFileSync(process.execPath, [generator], { cwd: root });
}

function filesBelow(directory, filename) {
  const result = [];
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    if (statSync(path).isDirectory())
      result.push(...filesBelow(path, filename));
    else if (entry === filename) result.push(path);
  }
  return result;
}
function digest(paths) {
  const hash = createHash("sha256");
  for (const path of paths) hash.update(readFileSync(path));
  return hash.digest("hex");
}

test("generated Apple project contains the complete Satchel integration", () => {
  apply();
  const mains = filesBelow(join(apple, "Sources"), "main.mm");
  assert.equal(mains.length, 1);
  const swift = join(dirname(mains[0]), "SatchelMobileIntegration.swift");
  const native = readFileSync(swift, "utf8");
  assert.doesNotMatch(readFileSync(mains[0], "utf8"), /satchel_mobile_install/);
  assert.match(native, /init_plugin_satchel_mail_notifications/);
  assert.match(readFileSync(join(dirname(mains[0]), "SatchelNotificationRouting.swift"), "utf8"), /__EXTRA__/);
  assert.match(native, /UNUserNotificationCenterDelegate/);
  assert.match(native, /satchel_mobile_frontend_ready/);
  assert.doesNotMatch(native, /relay|BGTaskScheduler|registerForRemoteNotifications/);
});

/**
 * The OAuth callback comes back through the custom scheme, and iOS only hands
 * it over if the bundle declares it. xcodegen writes Info.plist from
 * project.yml, so this is the file that has to carry the declaration.
 */
test("registers the OAuth callback scheme for the generated Info.plist", () => {
  apply();
  const project = readFileSync(join(apple, "project.yml"), "utf8");
  const config = JSON.parse(
    readFileSync(join(sourceRoot, "src-tauri/tauri.conf.json"), "utf8"),
  );
  const [domain] = config.plugins["deep-link"].mobile;

  assert.match(project, /CFBundleURLTypes:/);
  for (const scheme of domain.scheme) {
    assert.match(
      project,
      new RegExp(`CFBundleURLSchemes:\\n\\s+- ${scheme.replace(/\./g, "\\.")}`),
    );
  }
});

test("the generated-project patch is idempotent", () => {
  apply();
  const paths = [
    join(apple, "project.yml"),
    ...filesBelow(join(apple, "Sources"), "main.mm"),
    ...filesBelow(join(apple, "Sources"), "SatchelMobileIntegration.swift"),
    ...filesBelow(join(apple, "Sources"), "SatchelNotificationRouting.swift"),
  ];
  const before = digest(paths);
  apply();
  assert.equal(digest(paths), before);
});
