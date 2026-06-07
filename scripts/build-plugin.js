#!/usr/bin/env node
/* Build dist/system-monitor.plugin (zip). Bundles node_modules (systeminformation)
   so the plugin is self-contained. No npm dependencies in this script. */
"use strict";
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const root = path.join(__dirname, "..");
const dist = path.join(root, "dist");
const out = path.join(dist, "system-monitor.plugin");

if (!fs.existsSync(path.join(root, "node_modules", "systeminformation"))) {
  console.error("node_modules/systeminformation missing. Run `npm install` first.");
  process.exit(1);
}

fs.mkdirSync(dist, { recursive: true });
if (fs.existsSync(out)) fs.rmSync(out);

// Plugin payload (exclude dev-only files; include node_modules so it runs standalone).
const include = [".claude-plugin", ".mcp.json", "server", "skills", "node_modules", "README.md"];

try {
  if (process.platform === "win32") {
    const zip = path.join(dist, "system-monitor.zip");
    if (fs.existsSync(zip)) fs.rmSync(zip);
    const paths = include.map((p) => `'${path.join(root, p)}'`).join(",");
    execFileSync("powershell", ["-NoProfile", "-Command",
      `Compress-Archive -Path ${paths} -DestinationPath '${zip}' -Force`], { stdio: "inherit" });
    fs.renameSync(zip, out);
  } else {
    execFileSync("zip", ["-rq", out, ...include, "-x", "*.DS_Store"], { cwd: root, stdio: "inherit" });
  }
  const mb = (fs.statSync(out).size / 1e6).toFixed(2);
  console.log(`Built ${path.relative(root, out)} (${mb} MB)`);
} catch (e) {
  console.error("Build failed:", e.message);
  process.exit(1);
}
