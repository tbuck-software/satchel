import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/*
 * Assertions over mobile build configuration — Tauri and Android manifests,
 * Gradle, Make and dev scripts — plus one constant that has to read the same
 * in Rust, the UI and the platform registration. Configuration has no
 * behaviour to run, so its text is what there is to check. Split out of
 * mobileBuild.test.ts (ARCH-13).
 */
describe("mobile build configuration", () => {
  it.each(["mailto", "satchel"])("registers Satchel for %s: links on the desktop and on Android", (scheme) => {
    const tauriConfig = JSON.parse(
      readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
    ) as {
      plugins?: {
        "deep-link"?: {
          desktop?: { schemes?: string[] };
          mobile?: { scheme?: string[] }[];
        };
      };
    };
    const manifest = readFileSync(
      new URL(
        "../../src-tauri/gen/android/app/src/main/AndroidManifest.xml",
        import.meta.url,
      ),
      "utf8",
    );
    const deepLink = tauriConfig.plugins?.["deep-link"];

    expect(deepLink?.desktop?.schemes).toContain(scheme);
    expect(deepLink?.mobile?.some((entry) => entry.scheme?.includes(scheme))).toBe(true);
    expect(manifest).toMatch(
      new RegExp(
        `<intent-filter\\s*>(?:(?!<\\/intent-filter>)[\\s\\S])*android\\.intent\\.action\\.VIEW(?:(?!<\\/intent-filter>)[\\s\\S])*android:scheme="${scheme}"`,
      ),
    );
  });

  it("exposes the Tauri CLI script required by generated mobile build tasks", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../package.json", import.meta.url), "utf8"),
    ) as { scripts?: Record<string, string> };

    expect(packageJson.scripts?.tauri).toBe("tauri");
  });

  it("keeps the generated Android minimum SDK aligned with Tauri configuration", () => {
    const androidConfig = JSON.parse(
      readFileSync(
        new URL("../../src-tauri/tauri.android.conf.json", import.meta.url),
        "utf8",
      ),
    ) as { bundle?: { android?: { minSdkVersion?: number } } };
    const gradleBuild = readFileSync(
      new URL(
        "../../src-tauri/gen/android/app/build.gradle.kts",
        import.meta.url,
      ),
      "utf8",
    );

    expect(gradleBuild).toContain(
      `minSdk = ${androidConfig.bundle?.android?.minSdkVersion}`,
    );
  });

  it("allows the mobile shell to open trusted OAuth authorization URLs", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../src-tauri/capabilities/mobile.json", import.meta.url),
        "utf8",
      ),
    ) as { permissions?: string[] };

    expect(capability.permissions).toContain("opener:allow-open-url");
  });

  it("generates the Android OAuth intent filter from the deep-link configuration", () => {
    const config = JSON.parse(
      readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
    ) as {
      plugins?: {
        "deep-link"?: {
          mobile?: {
            scheme?: string[];
            host?: string;
            path?: string[];
            pathPattern?: string[];
            pathPrefix?: string[];
            pathSuffix?: string[];
          }[];
        };
      };
    };
    const manifest = readFileSync(
      new URL("../../src-tauri/gen/android/app/src/main/AndroidManifest.xml", import.meta.url),
      "utf8",
    );

    const domains = config.plugins?.["deep-link"]?.mobile ?? [];
    const oauth = domains.find((domain) => domain.host === "oauth");
    expect(oauth?.scheme).toEqual(["de.tbuck.satchel"]);
    expect(oauth?.pathPrefix).toEqual(["/callback"]);

    const data = (values: string[] | undefined, attribute: string) =>
      (values ?? [])
        .map((value) => `<data android:${attribute}="${value}" />`)
        .join("\n    ");
    // Mirrors `intent_filter` in the plugin's build.rs, joined the same way.
    const filter = domains
      .map((domain) => `<intent-filter >
    <action android:name="android.intent.action.VIEW" />
    <!-- ChromeOS ARC++ uses a different action for deep links -->
    <action android:name="org.chromium.arc.intent.action.VIEW" />
    <category android:name="android.intent.category.DEFAULT" />
    <category android:name="android.intent.category.BROWSABLE" />
    ${data(domain.scheme, "scheme")}
    ${domain.host ? `<data android:host="${domain.host}" />` : ""}
    ${data(domain.path, "path")}
    ${data(domain.pathPattern, "pathPattern")}
    ${data(domain.pathPrefix, "pathPrefix")}
    ${data(domain.pathSuffix, "pathSuffix")}
</intent-filter>`.trim())
      .join("\n");
    const marker = "<!-- DEEP LINK PLUGIN. AUTO-GENERATED. DO NOT REMOVE. -->";
    const indent = " ".repeat(12);
    const block = [marker, ...filter.split("\n"), marker]
      .map((line) => `${indent}${line}`)
      .join("\n");

    expect(manifest).toContain(block);
  });

  it("names the same OAuth callback URI in Rust, in the UI and in the registration", () => {
    const config = readFileSync(
      new URL("../../src-tauri/tauri.conf.json", import.meta.url),
      "utf8",
    );
    const core = readFileSync(
      new URL("../../crates/satchel-core/src/utils/oauth.rs", import.meta.url),
      "utf8",
    );
    const frontend = readFileSync(
      new URL("./mobileOAuth.ts", import.meta.url),
      "utf8",
    );

    expect(core).toContain(
      'pub const MOBILE_REDIRECT_URI: &str = "de.tbuck.satchel://oauth/callback";',
    );
    expect(frontend).toContain("const CALLBACK_SCHEME = 'de.tbuck.satchel:'");
    expect(frontend).toContain("const CALLBACK_HOST = 'oauth'");
    expect(frontend).toContain("const CALLBACK_PATH = '/callback'");
    expect(JSON.parse(config).plugins["deep-link"].mobile[0]).toEqual({
      scheme: ["de.tbuck.satchel"],
      host: "oauth",
      pathPrefix: ["/callback"],
    });
  });

  it("does not race Android's foreground Activity restore after an APK update", () => {
    const installScript = readFileSync(
      new URL("../../scripts/install-android.mjs", import.meta.url),
      "utf8",
    );

    expect(installScript).toContain("function isPackageForeground");
    expect(installScript).toContain("function waitForPackageRestore");
    expect(installScript).toMatch(
      /const wasForeground = shouldOpen && isPackageForeground[\s\S]*?install', '-r'[\s\S]*?wasForeground && waitForPackageRestore[\s\S]*?shell', 'monkey'/,
    );
  });

  it("exposes explicit desktop, Android, and iOS Make development targets", () => {
    const makefile = readFileSync(
      new URL("../../Makefile", import.meta.url),
      "utf8",
    );

    expect(makefile).toContain("dev:\n\tpnpm run tauri:dev");
    expect(makefile).toContain("dev-android:\n\tpnpm run tauri:android:dev");
    expect(makefile).toContain("dev-ios:\n\tpnpm run tauri:ios:dev");
    expect(makefile).toContain("make dev          Run the desktop app");
    expect(makefile).toContain("make dev-android  Run the Android app");
    expect(makefile).toContain("make dev-ios      Run the iOS app");
  });

  it("exposes the Vite development server to physical mobile devices", () => {
    const viteConfig = readFileSync(
      new URL("../../vite.config.ts", import.meta.url),
      "utf8",
    );

    expect(viteConfig).toContain("process.env.TAURI_DEV_HOST");
    expect(viteConfig).toContain("host: tauriDevHost || false");
  });

  it("uses the Android NDK ranlib for local development builds", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../package.json", import.meta.url), "utf8"),
    ) as { scripts?: Record<string, string> };
    const devScript = readFileSync(
      new URL("../../scripts/dev-android.mjs", import.meta.url),
      "utf8",
    );

    expect(packageJson.scripts?.["tauri:android:dev"]).toBe(
      "node ./scripts/dev-android.mjs",
    );
    expect(devScript).toContain("ANDROID_NDK_HOME");
    expect(devScript).toContain("RANLIB_${target}");
    expect(devScript).toContain("llvm-ranlib");
  });

  it("uses Satchel branding for Android notification, launcher and task icons", () => {
    const manifest = readFileSync(
      new URL("../../src-tauri/gen/android/app/src/main/AndroidManifest.xml", import.meta.url),
      "utf8",
    );
    const notificationIcon = readFileSync(
      new URL(
        "../../src-tauri/gen/android/app/src/main/res/drawable-hdpi/ic_notification.png",
        import.meta.url,
      ),
    );
    const macMenuBarIcon = readFileSync(
      new URL("../../src-tauri/icons/trayTemplate.png", import.meta.url),
    );
    const adaptiveIcons = ["ic_launcher", "ic_launcher_round"].map((name) =>
      readFileSync(
        new URL(
          `../../src-tauri/gen/android/app/src/main/res/mipmap-anydpi-v26/${name}.xml`,
          import.meta.url,
        ),
        "utf8",
      ),
    );
    const colors = readFileSync(
      new URL("../../src-tauri/gen/android/app/src/main/res/values/colors.xml", import.meta.url),
      "utf8",
    );
    const brandMark = readFileSync(
      new URL("../../public/branding/satchel-smile-check.svg", import.meta.url),
      "utf8",
    );
    const brandTile = /id="background"[^>]*fill="#([0-9A-F]{6})"/i.exec(brandMark)?.[1];

    expect(manifest).toContain('android:icon="@mipmap/ic_launcher"');
    expect(manifest).toContain('android:roundIcon="@mipmap/ic_launcher_round"');
    expect(notificationIcon).toEqual(macMenuBarIcon);
    // The vector mark sits inside the safe zone; a full-bleed PNG foreground
    // lost its corners to the launcher's circle mask.
    for (const adaptiveIcon of adaptiveIcons) {
      expect(adaptiveIcon).toContain('@color/satchel_icon_tile');
      expect(adaptiveIcon).toContain('@drawable/ic_launcher_mark');
    }
    expect(brandTile).toBeDefined();
    expect(colors).toContain(`<color name="satchel_icon_tile">#FF${brandTile}</color>`);
    expect(colors).not.toContain("purple_500");
  });

  it("allows explicit mobile exports without exposing all external storage", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../src-tauri/capabilities/mobile.json", import.meta.url),
        "utf8",
      ),
    ) as { permissions?: string[] };
    const filePaths = readFileSync(
      new URL(
        "../../src-tauri/gen/android/app/src/main/res/xml/file_paths.xml",
        import.meta.url,
      ),
      "utf8",
    );

    expect(capability.permissions).toContain("dialog:allow-save");
    expect(capability.permissions).toContain("fs:allow-write-file");
    expect(capability.permissions).toContain("fs:allow-remove");
    expect(filePaths).not.toContain("<external-path");
    expect(filePaths).toContain("<cache-path");
  });
});
