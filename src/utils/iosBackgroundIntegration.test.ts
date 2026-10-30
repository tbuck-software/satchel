import { execFileSync } from "node:child_process";
import {
  chmodSync,
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
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

const swiftTemplate = ["SatchelMobileIntegration.swift", "SatchelNotificationRouting.swift"]
  .map(name => readFileSync(new URL(`../../src-tauri/ios/${name}`, import.meta.url), "utf8"))
  .join("\n");

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
    const swiftPath = join(
      root,
      "src-tauri/gen/apple/Sources/satchel/SatchelMobileIntegration.swift",
    );
    const routingPath = join(root, "src-tauri/gen/apple/Sources/satchel/SatchelNotificationRouting.swift");
    const first = [projectPath, mainPath, swiftPath, routingPath].map((path) =>
      readFileSync(path, "utf8"),
    );

    // project.yml is left alone: no background modes, no APNs entitlement.
    expect(first[0]).not.toContain("UIBackgroundModes");
    expect(first[0]).not.toContain("BGTaskSchedulerPermittedIdentifiers");
    expect(first[1]).not.toContain("satchel_mobile_install");
    expect(readFileSync(join(root, "src-tauri/gen/apple/Sources/satchel/SatchelNotificationRouting.swift"), "utf8")).toContain("__EXTRA__");
    expect(first[2]).toContain("UNUserNotificationCenterDelegate");

    apply(root);
    expect(
      [projectPath, mainPath, swiftPath, routingPath].map((path) => readFileSync(path, "utf8")),
    ).toEqual(first);
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
    expect(bridge).toContain("mobile:notification-route");
    expect(bridge).toContain("libc::dlsym(libc::RTLD_DEFAULT");
    expect(bridge).not.toContain('unsafe extern "C" {');
    expect(workflow).toContain("node scripts/apply-ios-integration.mjs");
    expect(workflow).toContain(
      "node --test tests/ios-integration-generator.node.mjs",
    );
    expect(workflow).toContain("pnpm tauri ios build");
    expect(workflow).toContain('"dyld_info", "-exports"');
    expect(workflow).toContain("init_plugin_satchel_mail_notifications");
  });
});
