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
    new URL(
      "../../src-tauri/ios/SatchelMobileIntegration.swift",
      import.meta.url,
    ),
    join(root, "src-tauri/ios/SatchelMobileIntegration.swift"),
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

const swiftTemplate = readFileSync(
  new URL("../../src-tauri/ios/SatchelMobileIntegration.swift", import.meta.url),
  "utf8",
);

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

  it("keeps a notification tap pending until the frontend listener is ready", () => {
    expect(swiftTemplate).toContain("private var frontendReady = false");
    expect(swiftTemplate).toContain("guard frontendReady, let route = pendingRoute");
    expect(swiftTemplate).toContain('@_cdecl("satchel_mobile_frontend_ready")');
    expect(swiftTemplate).toContain('"type": "notification-tap"');
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
    const first = [projectPath, mainPath, swiftPath].map((path) =>
      readFileSync(path, "utf8"),
    );

    // project.yml is left alone: no background modes, no APNs entitlement.
    expect(first[0]).not.toContain("UIBackgroundModes");
    expect(first[0]).not.toContain("BGTaskSchedulerPermittedIdentifiers");
    expect(first[1]).toContain("satchel_mobile_install();");
    expect(first[2]).toContain("UNUserNotificationCenterDelegate");
    expect(first[2]).toContain("threadId");

    apply(root);
    expect(
      [projectPath, mainPath, swiftPath].map((path) => readFileSync(path, "utf8")),
    ).toEqual(first);
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
  });
});
