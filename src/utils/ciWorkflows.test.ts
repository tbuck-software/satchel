import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/*
 * Assertions over CI workflow YAML. Text checks are the honest tool here: the
 * workflows run only on GitHub, so what can be verified locally is that they
 * still say the right thing. Split out of mobileBuild.test.ts (ARCH-13), which
 * mixed these with regular expressions over application source.
 */
describe("CI workflows", () => {
  it("runs public contributions only on GitHub-hosted runners", () => {
    for (const name of ["ci", "security-audit", "fuzz"]) {
      const workflow = readFileSync(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), "utf8");
      expect(workflow).not.toContain("self-hosted");
      expect(workflow).not.toContain("head.repo.full_name");
    }
  });

  it("applies the macOS sqlx macro workaround to every local Cargo invocation", () => {
    const manifest = readFileSync(new URL("../../Cargo.toml", import.meta.url), "utf8");
    for (const profile of ["dev", "test"]) {
      expect(manifest).toContain(`[profile.${profile}.package.sqlx-macros]\ndebug = "line-tables-only"`);
    }
    const ci = readFileSync(new URL("../../.github/workflows/ci.yml", import.meta.url), "utf8");
    expect(ci).not.toContain("--config .cargo/ci-macos.toml");
  });

  it("uses the pinned toolchain and public dependencies for mobile builds", () => {
    for (const name of ["android", "android-release", "ios"]) {
      const workflow = readFileSync(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), "utf8");
      expect(workflow).not.toContain("MAIL_CORE_DEPLOY_KEY");
      expect(workflow).not.toContain("mail-core.git");
      expect(workflow).not.toContain("core.sshCommand");
      expect(workflow).not.toContain("dtolnay/rust-toolchain@stable");
      expect(workflow).toContain("uses: actions-rust-lang/setup-rust-toolchain@v2");
      expect(workflow).not.toMatch(/toolchain:\s*(stable|nightly)/);
      expect(workflow).toContain("target:");
    }
  });

  it("pins OpenSSL ranlib discovery to the configured NDK", () => {
    const android = readFileSync(
      new URL("../../.github/workflows/android.yml", import.meta.url),
      "utf8",
    );
    const release = readFileSync(
      new URL("../../.github/workflows/android-release.yml", import.meta.url),
      "utf8",
    );
    expect(android).toContain("ANDROID_NDK_HOME");
    expect(android).toContain("ANDROID_NDK_ROOT");
    expect(android).toContain("RANLIB_x86_64_linux_android");
    expect(release).toContain("RANLIB_aarch64_linux_android");
  });

  it("builds mobile changes on PRs and runs Android runtime checks nightly", () => {
    for (const name of ["android", "ios"]) {
      const workflow = readFileSync(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), "utf8");
      expect(workflow).toContain("pull_request:");
      for (const path of ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates/satchel-core/**", "src-tauri/**"]) {
        expect(workflow).toContain(`- '${path}'`);
      }
      expect(workflow).toContain("workflow_dispatch:");
      expect(workflow).toContain('tags:\n      - "v*"');
    }
    const android = readFileSync(new URL("../../.github/workflows/android.yml", import.meta.url), "utf8");
    expect(android).toContain('cron: "35 2 * * *"');
    expect(android).toContain("Install and launch APK on Android 8\n        if: github.event_name != 'pull_request'");
    expect(android).not.toContain("Build x86_64 debug APK\n        if:");
  });

  it("builds the iOS simulator project on a macOS CI runner", () => {
    const workflow = readFileSync(
      new URL("../../.github/workflows/ios.yml", import.meta.url),
      "utf8",
    );

    expect(workflow).toContain("runs-on: macos-15");
    expect(workflow).toContain("aarch64-apple-ios-sim");
    expect(workflow).toContain("tauri ios init --ci");
    expect(workflow).toContain(
      "tauri ios build --debug --target aarch64-sim --ci",
    );
  });

  it("builds and launches the Android APK in CI with hardware acceleration", () => {
    const workflow = readFileSync(
      new URL("../../.github/workflows/android.yml", import.meta.url),
      "utf8",
    );

    expect(workflow).toContain("runs-on: ubuntu-24.04");
    expect(workflow).toContain("aarch64-linux-android");
    expect(workflow).toContain("x86_64-linux-android");
    expect(workflow).toContain(
      "tauri android build --debug --apk --target x86_64 --ci",
    );
    expect(workflow).toContain("reactivecircus/android-emulator-runner@v2");
    expect(workflow).toContain("Reclaim build disk before emulator");
    expect(workflow).toContain('rm -rf "$ANDROID_HOME/ndk"');
    expect(workflow.indexOf("Build x86_64 debug APK")).toBeLessThan(
      workflow.indexOf("Reclaim build disk before emulator"),
    );
    expect(workflow.indexOf("Reclaim build disk before emulator")).toBeLessThan(
      workflow.indexOf("Install and launch APK on Android 8"),
    );
    expect(workflow).toContain("target: google_apis");
    expect(workflow).toContain(
      "am start -W -n de.tbuck.satchel/.MainActivity",
    );
    expect(workflow).toContain("de.tbuck.satchel://oauth/callback");
    expect(workflow).not.toContain("state=ci-invalid&code=ci-invalid");
    expect(workflow).not.toContain("APK=src-tauri/");
    expect(workflow).toContain(
      "adb install --no-streaming -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk",
    );
    expect(workflow).toMatch(
      /android\.intent\.action\.VIEW[^\n]*de\.tbuck\.satchel:\/\/oauth\/callback/,
    );
  });

  it("keeps Android release signing secret-driven and builds an AAB manually", () => {
    const gradle = readFileSync(
      new URL(
        "../../src-tauri/gen/android/app/build.gradle.kts",
        import.meta.url,
      ),
      "utf8",
    );
    const gitignore = readFileSync(
      new URL("../../src-tauri/gen/android/.gitignore", import.meta.url),
      "utf8",
    );
    const workflow = readFileSync(
      new URL("../../.github/workflows/android-release.yml", import.meta.url),
      "utf8",
    );

    expect(gradle).toContain("keystore.properties");
    expect(gradle).toContain("signingConfigs");
    expect(gradle).toContain("signingConfig = releaseSigning");
    expect(gitignore).toContain("keystore.properties");
    expect(workflow).toContain("workflow_dispatch");
    expect(workflow).toContain("secrets.ANDROID_KEY_BASE64");
    expect(workflow).toContain(
      "tauri android build --aab --target aarch64 --ci",
    );
  });
});
