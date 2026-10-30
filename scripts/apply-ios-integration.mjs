#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import process from "node:process";

const root = resolve(process.cwd());
const apple = join(root, "src-tauri", "gen", "apple");
const projectPath = join(apple, "project.yml");
const swiftSources = [
  "SatchelMobileIntegration.swift",
  "SatchelNotificationRouting.swift",
];
const expectedBundleId = "de.tbuck.satchel";

/**
 * @param {string} message
 */
function fail(message) {
  throw new Error(`iOS integration: ${message}`);
}
/**
 * @param {string} path
 */
function readRequired(path) {
  if (!existsSync(path))
    fail(`missing ${relative(root, path)}; run tauri ios init --ci first`);
  return readFileSync(path, "utf8");
}
/**
 * @param {string} directory
 * @param {string} filename
 * @returns {string[]}
 */
function filesBelow(directory, filename) {
  const found = [];
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    const stat = statSync(path);
    if (stat.isDirectory()) found.push(...filesBelow(path, filename));
    else if (entry === filename) found.push(path);
  }
  return found;
}
/** The custom URL schemes `plugins.deep-link` registers for mobile. */
function deepLinkSchemes() {
  /** @type {{ plugins?: { "deep-link"?: { mobile?: Array<{ scheme?: string | string[] }> } } }} */
  const config = JSON.parse(
    readRequired(join(root, "src-tauri", "tauri.conf.json")),
  );
  const schemes = (config.plugins?.["deep-link"]?.mobile ?? [])
    .flatMap((domain) => domain.scheme ?? [])
    .filter((scheme) => scheme !== "http" && scheme !== "https");
  if (schemes.length === 0)
    fail("tauri.conf.json registers no custom deep-link scheme");
  return [...new Set(schemes)];
}

/**
 * Declare the OAuth callback scheme where xcodegen will read it.
 *
 * xcodegen regenerates Info.plist from project.yml, so anything the deep-link
 * plugin writes into the plist directly is gone the next time the project is
 * generated — and without CFBundleURLTypes iOS never hands the callback back
 * to the app, leaving the OAuth flow to time out.
 */
/**
 * @param {string} text
 * @param {string[]} schemes
 */
function withUrlTypes(text, schemes) {
  if (text.includes("CFBundleURLTypes")) return text;
  const anchor = /\n( *)path: [^\n]*Info\.plist\n *properties:\n/;
  const match = text.match(anchor);
  if (!match) fail("no generated Info.plist properties block in project.yml");
  const indent = " ".repeat(match[1].length + 2);
  const block = [
    `${indent}CFBundleURLTypes:`,
    `${indent}  - CFBundleURLName: ${schemes[0]}`,
    `${indent}    CFBundleURLSchemes:`,
    ...schemes.map((scheme) => `${indent}      - ${scheme}`),
    "",
  ].join("\n");
  return text.replace(anchor, (found) => `${found}${block}`);
}

const schemes = deepLinkSchemes();
let project = readRequired(projectPath);
if (!project.includes(`PRODUCT_BUNDLE_IDENTIFIER: ${expectedBundleId}`)) {
  fail(`unexpected bundle identifier; expected ${expectedBundleId}`);
}
project = withUrlTypes(project, schemes);
writeFileSync(projectPath, project);

const mainFiles = filesBelow(join(apple, "Sources"), "main.mm");
if (mainFiles.length !== 1)
  fail(`expected one generated main.mm, found ${mainFiles.length}`);
const mainPath = mainFiles[0];
let main = readRequired(mainPath);
// Remove the pre-plugin installation injected by earlier versions. Plugin
// registration now owns installation, after NotificationManager.init.
main = main.replace('\nextern "C" void satchel_mobile_install(void);\n', "");
main = main.replace("\tsatchel_mobile_install();\n", "");
if (main.includes("satchel_mobile_install"))
  fail("unexpected legacy delegate installation");
// Mach-O's `used` attribute retains this table and its referenced public Swift
// C entry points even with dead stripping/LTO. It never calls the initializer:
// plugin setup still installs the delegate after the notification plugin.
const hooksStart = "// Satchel notification native hooks: begin";
const hooksEnd = "// Satchel notification native hooks: end";
if (main.includes(hooksStart) !== main.includes(hooksEnd)
    || main.split(hooksStart).length > 2 || main.split(hooksEnd).length > 2
    || (main.includes(hooksStart) && main.indexOf(hooksEnd) < main.indexOf(hooksStart)))
  fail("unexpected native hook retention block");
if (main.includes(hooksStart)) {
  const start = main.indexOf(hooksStart);
  const end = main.indexOf(hooksEnd) + hooksEnd.length;
  main = main.slice(0, start) + main.slice(end).replace(/^\n/, "");
}
const include = '#include "bindings/bindings.h"\n';
if (main.split(include).length !== 2)
  fail("unexpected generated main.mm bindings include");
const hooks = `${hooksStart}
#include <stdint.h>
extern "C" void *init_plugin_satchel_mail_notifications(void);
extern "C" uint32_t satchel_mobile_frontend_ready(uint32_t, bool);
extern "C" void satchel_mobile_response_handled(const char *);
__attribute__((used)) static const struct {
  void *(*initialize)(void);
  uint32_t (*lifecycle)(uint32_t, bool);
  void (*handled)(const char *);
} satchel_notification_native_hooks = {
  init_plugin_satchel_mail_notifications,
  satchel_mobile_frontend_ready,
  satchel_mobile_response_handled,
};
${hooksEnd}
`;
main = main.replace(include, include + hooks);
writeFileSync(mainPath, main);

for (const filename of swiftSources) {
  const template = join(root, "src-tauri", "ios", filename);
  readRequired(template);
  copyFileSync(template, join(dirname(mainPath), filename));
}

// The Swift file is new to the generated project; xcodegen has to see it.
const xcodeProjects = readdirSync(apple).filter((entry) =>
  entry.endsWith(".xcodeproj"),
);
if (xcodeProjects.length > 1) {
  fail(
    `expected at most one generated Xcode project, found ${xcodeProjects.length}`,
  );
}
if (xcodeProjects.length === 1) {
  execFileSync(
    "xcodegen",
    ["generate", "--spec", projectPath, "--project", apple],
    { cwd: apple, stdio: "inherit" },
  );
}

for (const required of [
  "init_plugin_satchel_mail_notifications",
  "UNUserNotificationCenterDelegate",
  "satchel_mobile_frontend_ready",
  "satchel_mobile_response_handled",
]) {
  const all = swiftSources
    .map((name) => readRequired(join(dirname(mainPath), name)))
    .join("\n");
  if (!all.includes(required)) fail(`verification failed for ${required}`);
}

for (const scheme of schemes) {
  if (!project.includes(`- ${scheme}`))
    fail(`project.yml does not register the ${scheme} URL scheme`);
}
// Present once xcodegen has run; `tauri ios init` on its own leaves none.
const plistPath = join(apple, project.match(/path: ([^\n]*Info\.plist)/)[1]);
if (existsSync(plistPath)) {
  const plist = readFileSync(plistPath, "utf8");
  for (const key of ["CFBundleURLTypes", "CFBundleURLSchemes"]) {
    if (!plist.includes(key))
      fail(`generated Info.plist is missing ${key}; xcodegen did not pick it up`);
  }
}

console.log(`Applied Satchel iOS integration to ${relative(root, apple)}`);
