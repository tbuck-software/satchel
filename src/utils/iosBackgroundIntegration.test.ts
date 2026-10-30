import { execFileSync } from "node:child_process";
import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

function generatedAppleFixture(): string {
  const root = mkdtempSync(join(tmpdir(), "satchel-ios-integration-"));
  cpSync(
    new URL("./fixtures/generated-apple", import.meta.url),
    join(root, "src-tauri/gen/apple"),
    { recursive: true },
  );
  mkdirSync(join(root, "src-tauri/ios"), { recursive: true });
  cpSync(
    new URL("../../src-tauri/ios", import.meta.url),
    join(root, "src-tauri/ios"),
    { recursive: true },
  );
  // The script reads the deep link schemes from here, so the fixture is
  // incomplete without it — the real config, not a stand-in, or the test
  // would stop noticing when the two drift apart.
  cpSync(
    new URL("../../src-tauri/tauri.conf.json", import.meta.url),
    join(root, "src-tauri/tauri.conf.json"),
  );
  return root;
}

function apply(root: string, extraEnv: Record<string, string> = {}): void {
  execFileSync(
    process.execPath,
    [
      // fileURLToPath, not .pathname: a checkout under a directory with a
      // space arrives as "%20" otherwise and node cannot find the script.
      fileURLToPath(
        new URL("../../scripts/apply-ios-integration.mjs", import.meta.url),
      ),
    ],
    {
      cwd: root,
      stdio: "pipe",
      env: { ...process.env, ...extraEnv },
    },
  );
}

const swiftSources = ["SatchelMobileIntegration.swift", "SatchelNotificationRouting.swift"];
const swiftTemplate = swiftSources
  .map(name => readFileSync(new URL(`../../src-tauri/ios/Sources/SatchelIOS/${name}`, import.meta.url), "utf8"))
  .join("\n");

function swiftFilesBelow(directory: string): string[] {
  return readdirSync(directory, { recursive: true, encoding: "utf8" })
    .filter((entry) => entry.endsWith(".swift"))
    .map((entry) => join(directory, entry));
}

describe("generated iOS native integration", () => {
  it("regenerates an existing Xcode project after installing the Swift source", () => {
    const root = generatedAppleFixture();
    const project = join(root, "src-tauri/gen/apple/satchel.xcodeproj");
    mkdirSync(project, { recursive: true });
    writeFileSync(join(project, "project.pbxproj"), "// generated fixture\n");
    const bin = join(root, "bin");
    mkdirSync(bin);
    const argsPath = join(root, "xcodegen-args.txt");
    const xcodegen = join(bin, "xcodegen");
    writeFileSync(
      xcodegen,
      `#!/bin/sh
printf "%s\\n" "$@" > "$SATCHEL_XCODEGEN_ARGS"
`,
    );
    chmodSync(xcodegen, 0o755);

    apply(root, {
      PATH: `${bin}:${process.env.PATH ?? ""}`,
      SATCHEL_XCODEGEN_ARGS: argsPath,
    });

    const args = readFileSync(argsPath, "utf8").trim().split("\n");
    const canonicalRoot = realpathSync(root);
    expect(args).toEqual([
      "generate",
      "--spec",
      join(canonicalRoot, "src-tauri/gen/apple/project.yml"),
      "--project",
      join(canonicalRoot, "src-tauri/gen/apple"),
    ]);
    const workflow = readFileSync(
      new URL("../../.github/workflows/ios.yml", import.meta.url),
      "utf8",
    );
    expect(workflow).toContain("brew install xcodegen");
  });

  /**
   * `tauri ios init` generates an app target that links libapp.a and nothing
   * else: the `Tauri` Swift module exists only inside the Rust build, where
   * tauri's build script links it. A Swift file in that target that imports it
   * fails with "no such module 'Tauri'", so the plugin is a Swift package that
   * the app's build script links, the way plugin crates link theirs.
   */
  it("builds the Tauri plugin as a linked Swift package, outside the Xcode target", () => {
    const root = generatedAppleFixture();
    apply(root);
    const apple = join(root, "src-tauri/gen/apple");
    for (const path of swiftFilesBelow(apple)) {
      expect(readFileSync(path, "utf8"), path).not.toMatch(/^\s*import\s+Tauri\b/m);
    }
    expect(swiftTemplate).toMatch(/^import Tauri$/m);
    expect(swiftTemplate).toMatch(/: Plugin \{/);

    const manifest = readFileSync(new URL("../../src-tauri/ios/Package.swift", import.meta.url), "utf8");
    expect(manifest).toContain('.package(name: "Tauri", path: "../.tauri/tauri-api")');
    expect(manifest).toMatch(/\.library\(\s*name: "SatchelIOS",\s*type: \.static,\s*targets: \["SatchelIOS"\]\)/);
    expect(manifest).toContain('path: "Sources/SatchelIOS"');
    const build = readFileSync(new URL("../../src-tauri/build.rs", import.meta.url), "utf8");
    expect(build).toContain('tauri_utils::build::link_apple_library("SatchelIOS"');
    expect(build).toContain("DEP_TAURI_IOS_LIBRARY_PATH");
    const ignored = readFileSync(new URL("../../.gitignore", import.meta.url), "utf8");
    expect(ignored).toMatch(/^src-tauri\/\.tauri\/$/m);
  });

  it("removes the Swift copies an earlier generation put into the Xcode target", () => {
    const root = generatedAppleFixture();
    const sources = join(root, "src-tauri/gen/apple/Sources/satchel");
    for (const name of swiftSources) writeFileSync(join(sources, name), "import Tauri\n");
    apply(root);
    for (const name of swiftSources) expect(existsSync(join(sources, name)), name).toBe(false);
  });

  it("talks to no server: no push registration, background tasks or keychain", () => {
    expect(swiftTemplate).not.toMatch(/relay/i);
    expect(swiftTemplate).not.toContain("registerForRemoteNotifications");
    expect(swiftTemplate).not.toContain("BGTaskScheduler");
    expect(swiftTemplate).not.toContain("URLSession");
    expect(swiftTemplate).not.toContain("SecItem");
  });

  it("applies the integration to a fresh Tauri Apple project and is idempotent", () => {
    const root = generatedAppleFixture();
    apply(root);
    const projectPath = join(root, "src-tauri/gen/apple/project.yml");
    const mainPath = join(root, "src-tauri/gen/apple/Sources/satchel/main.mm");
    const first = [projectPath, mainPath].map((path) => readFileSync(path, "utf8"));

    // project.yml is left alone: no background modes, no APNs entitlement.
    expect(first[0]).not.toContain("UIBackgroundModes");
    expect(first[0]).not.toContain("BGTaskSchedulerPermittedIdentifiers");
    expect(first[1]).not.toContain("satchel_mobile_install");
    expect(first[1]).toContain("satchel_notification_native_hooks");
    expect(swiftTemplate).toContain("__EXTRA__");
    expect(swiftTemplate).toContain("UNUserNotificationCenterDelegate");

    apply(root);
    expect([projectPath, mainPath].map((path) => readFileSync(path, "utf8"))).toEqual(first);
  });

  it("removes an earlier pre-plugin delegate installation and remains idempotent", () => {
    const root = generatedAppleFixture();
    const mainPath = join(root, "src-tauri/gen/apple/Sources/satchel/main.mm");
    const original = readFileSync(mainPath, "utf8");
    apply(root);
    const expected = readFileSync(mainPath, "utf8");
    writeFileSync(mainPath, original
      .replace('#include "bindings/bindings.h"\n', '#include "bindings/bindings.h"\n\nextern "C" void satchel_mobile_install(void);\n')
      .replace('int main(int argc, char * argv[]) {\n', 'int main(int argc, char * argv[]) {\n\tsatchel_mobile_install();\n'));
    apply(root);
    expect(readFileSync(mainPath, "utf8")).toBe(expected);
    apply(root);
    expect(readFileSync(mainPath, "utf8")).toBe(expected);
  });

  it("registers the native owner synchronously after the notification plugin", () => {
    const app = readFileSync(new URL("../../src-tauri/src/lib.rs", import.meta.url), "utf8");
    const bridge = readFileSync(new URL("../../src-tauri/src/commands/mobile_notifications.rs", import.meta.url), "utf8");
    const notificationPlugin = app.indexOf(".plugin(tauri_plugin_notification::init())");
    const mailPlugin = app.indexOf(".plugin(commands::mobile_notifications::init())");
    expect(notificationPlugin).toBeGreaterThan(-1);
    expect(mailPlugin).toBeGreaterThan(notificationPlugin);
    expect(bridge).toContain("api.register_ios_plugin(initialize_ios_mail_plugin)?");
    expect(bridge).toContain('ios_native_symbol::<IosPluginInitializer>(c"init_plugin_satchel_mail_notifications")');
    expect(bridge).not.toContain("tauri::ios_plugin_binding!");
    expect(bridge).toContain(".map_err(std::io::Error::other)?");
    expect(swiftTemplate).toContain("SatchelMobileIntegration.shared.install()");
  });

  it("fails closed instead of patching an unexpected bundle identifier", () => {
    const root = generatedAppleFixture();
    const projectPath = join(root, "src-tauri/gen/apple/project.yml");
    writeFileSync(
      projectPath,
      readFileSync(projectPath, "utf8").replaceAll(
        "de.tbuck.satchel",
        "example.invalid.app",
      ),
    );
    expect(() => apply(root)).toThrow();
  });

  it("fails closed on an unrecognized native hook retention block", () => {
    const root = generatedAppleFixture();
    const mainPath = join(root, "src-tauri/gen/apple/Sources/satchel/main.mm");
    writeFileSync(mainPath, readFileSync(mainPath, "utf8") + "\n// Satchel notification native hooks: begin\n");
    expect(() => apply(root)).toThrow();
  });

  it("keeps native source, bridge commands, and CI steps reproducible", () => {
    const bridge = readFileSync(
      new URL("../../src-tauri/src/commands/mobile_notifications.rs", import.meta.url),
      "utf8",
    );
    const workflow = readFileSync(
      new URL("../../.github/workflows/ios.yml", import.meta.url),
      "utf8",
    );
    expect(swiftTemplate).toContain('@_silgen_name("satchel_mobile_event")');
    expect(bridge).toContain("pub fn mobile_notifications_frontend_ready");
    // The ACK is a command with an ACL entry, not an untyped event.
    expect(bridge).toContain("pub fn mobile_notification_response_handled");
    expect(bridge).not.toContain("mobile:notification-response-handled");
    expect(bridge).toContain("mobile:notification-route");
    expect(bridge).toContain("libc::dlsym(libc::RTLD_DEFAULT");
    expect(bridge).not.toContain('unsafe extern "C" {');
    expect(workflow).toContain("node scripts/apply-ios-integration.mjs");
    expect(workflow).toContain(
      "node --test tests/ios-integration-generator.node.mjs",
    );
    expect(workflow).toContain("pnpm tauri ios build");
    // The export check finds the built image through xcodebuild's settings.
    expect(workflow).toContain("node scripts/verify-ios-hook-exports.mjs");
    expect(workflow).toContain("-scheme satchel_iOS -sdk iphonesimulator -configuration debug");
  });
});
